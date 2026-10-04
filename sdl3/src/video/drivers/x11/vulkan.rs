// Rust translation of src/video/x11/SDL_x11vulkan.c and SDL_x11vulkan.h
// from Simple DirectMedia Layer, with the parts of
// src/video/SDL_vulkan_utils.c they use.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Vulkan surfaces for X11 windows: through `VK_KHR_xlib_surface`, or
//! `VK_KHR_xcb_surface` with the XCB connection of the display (from
//! libX11-xcb, loaded at run time like the Vulkan loader).

use std::ffi::{c_char, c_ulong, c_void, CStr, CString};

use super::sys::*;
use super::video::X11Video;
use super::window::with_x11_window;
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::hints;
use crate::loadso::SharedObject;

#[cfg(target_os = "openbsd")]
const DEFAULT_VULKAN: &str = "libvulkan.so";
#[cfg(target_os = "openbsd")]
const DEFAULT_X11_XCB: &str = "libX11-xcb.so";
#[cfg(not(target_os = "openbsd"))]
const DEFAULT_VULKAN: &str = "libvulkan.so.1";
#[cfg(not(target_os = "openbsd"))]
const DEFAULT_X11_XCB: &str = "libX11-xcb.so.1";

const VK_KHR_SURFACE_EXTENSION_NAME: &str = "VK_KHR_surface";
const VK_KHR_XCB_SURFACE_EXTENSION_NAME: &str = "VK_KHR_xcb_surface";
const VK_KHR_XLIB_SURFACE_EXTENSION_NAME: &str = "VK_KHR_xlib_surface";

type VkResult = i32;
const VK_SUCCESS: VkResult = 0;
const VK_ERROR_INCOMPATIBLE_DRIVER: VkResult = -9;

const VK_STRUCTURE_TYPE_XLIB_SURFACE_CREATE_INFO_KHR: i32 = 1000004000;
const VK_STRUCTURE_TYPE_XCB_SURFACE_CREATE_INFO_KHR: i32 = 1000005000;

/// `VkExtensionProperties`.
#[repr(C)]
struct VkExtensionProperties {
    extension_name: [c_char; 256],
    spec_version: u32,
}

/// `VkXcbSurfaceCreateInfoKHR`.
#[repr(C)]
struct VkXcbSurfaceCreateInfoKHR {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    connection: *mut c_void,
    window: u32,
}

/// `VkXlibSurfaceCreateInfoKHR`.
#[repr(C)]
struct VkXlibSurfaceCreateInfoKHR {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    dpy: *mut Display,
    window: Window,
}

type PfnVkGetInstanceProcAddr = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;
type PfnVkEnumerateInstanceExtensionProperties =
    unsafe extern "C" fn(*const c_char, *mut u32, *mut VkExtensionProperties) -> VkResult;
type PfnVkCreateXcbSurfaceKHR = unsafe extern "C" fn(
    *mut c_void,
    *const VkXcbSurfaceCreateInfoKHR,
    *const c_void,
    *mut u64,
) -> VkResult;
type PfnVkCreateXlibSurfaceKHR = unsafe extern "C" fn(
    *mut c_void,
    *const VkXlibSurfaceCreateInfoKHR,
    *const c_void,
    *mut u64,
) -> VkResult;
type PfnVkDestroySurfaceKHR = unsafe extern "C" fn(*mut c_void, u64, *const c_void);
type PfnVkGetPhysicalDeviceXcbPresentationSupportKHR =
    unsafe extern "C" fn(*mut c_void, u32, *mut c_void, u32) -> u32;
type PfnVkGetPhysicalDeviceXlibPresentationSupportKHR =
    unsafe extern "C" fn(*mut c_void, u32, *mut Display, VisualID) -> u32;
type PfnXGetXCBConnection = unsafe extern "C" fn(*mut Display) -> *mut c_void;

/// The Vulkan state of the device: the loader (`vulkan_config.loader_handle`
/// and its `vkGetInstanceProcAddr`) and the `vulkan_xlib_xcb_library` /
/// `vulkan_XGetXCBConnection` members of `SDL_VideoData`.
#[derive(Default)]
pub(crate) struct VulkanData {
    loader_handle: Option<SharedObject>,
    vk_get_instance_proc_addr: Option<PfnVkGetInstanceProcAddr>,
    vulkan_xlib_xcb_library: Option<SharedObject>,
    vulkan_x_get_xcb_connection: Option<PfnXGetXCBConnection>,
    /// `vulkan_xlib_xcb_library != NULL`, which upstream doesn't reset when
    /// the library is unloaded.
    uses_xcb: bool,
}

impl VulkanData {
    /// Whether the loader is loaded (`vulkan_config.loader_handle`).
    pub(crate) fn loader_loaded(&self) -> bool {
        self.loader_handle.is_some()
    }
}

/// The name of a `VkResult`. Translation of `SDL_Vulkan_GetResultString()`.
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

/// The names of the instance extensions. Translation of
/// `SDL_Vulkan_CreateInstanceExtensionsList()`.
fn vulkan_create_instance_extensions_list(
    vk_enumerate_instance_extension_properties: PfnVkEnumerateInstanceExtensionProperties,
) -> Result<Vec<String>> {
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
        return Err(Error::new(format!(
            "Getting Vulkan extensions failed: vkEnumerateInstanceExtensionProperties returned {}({})",
            vulkan_get_result_string(rc),
            rc
        )));
    }

    let mut result: Vec<VkExtensionProperties> = (0..count.max(1))
        .map(|_| VkExtensionProperties {
            extension_name: [0; 256],
            spec_version: 0,
        })
        .collect();

    // SAFETY: the array has room for `count` properties.
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
            // SAFETY: the names are NUL-terminated within their 256 bytes
            // (the arrays started zeroed).
            unsafe { CStr::from_ptr(e.extension_name.as_ptr()) }
                .to_string_lossy()
                .into_owned()
        })
        .collect())
}

/// Look up an instance function (`vkGetInstanceProcAddr(instance, name)`).
///
/// # Safety
///
/// `F` must be the function's pointer type.
unsafe fn instance_function<F: Copy>(
    get: PfnVkGetInstanceProcAddr,
    instance: usize,
    name: &str,
) -> Option<F> {
    let name = CString::new(name).ok()?;
    // SAFETY: the loader's vkGetInstanceProcAddr; the name is NUL-terminated.
    let p = unsafe { get(instance as *mut c_void, name.as_ptr()) };
    if p.is_null() {
        return Option::None;
    }
    const { assert!(std::mem::size_of::<F>() == std::mem::size_of::<*mut c_void>()) };
    // SAFETY: the caller's contract; F is a function pointer.
    Some(unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p) })
}

impl X11Video {
    /// Translation of `X11_Vulkan_LoadLibrary()`.
    pub(crate) fn x11_vulkan_load_library(&self, path: Option<&str>) -> Result<()> {
        let mut has_surface_extension = false;
        let mut has_xlib_surface_extension = false;
        let mut has_xcb_surface_extension = false;
        if self.with_data(|d| d.vulkan.loader_handle.is_some()) {
            return Err(Error::new("Vulkan already loaded"));
        }

        // Load the Vulkan loader library
        let hint = hints::get(hints::VULKAN_LIBRARY);
        let path = path
            .map(str::to_owned)
            .or(hint)
            .unwrap_or_else(|| DEFAULT_VULKAN.to_owned());
        let loader_handle = SharedObject::load(&path)?;
        // (vulkan_config.loader_path is kept by the video core)
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
            // (fail: unloading the loader)
            return Err(Error::new(
                "vkEnumerateInstanceExtensionProperties not found",
            ));
        };
        let extensions =
            vulkan_create_instance_extensions_list(vk_enumerate_instance_extension_properties)?;
        for extension in &extensions {
            if extension == VK_KHR_SURFACE_EXTENSION_NAME {
                has_surface_extension = true;
            } else if extension == VK_KHR_XCB_SURFACE_EXTENSION_NAME {
                has_xcb_surface_extension = true;
            } else if extension == VK_KHR_XLIB_SURFACE_EXTENSION_NAME {
                has_xlib_surface_extension = true;
            }
        }
        if !has_surface_extension {
            return Err(Error::new(format!(
                "Installed Vulkan doesn't implement the {VK_KHR_SURFACE_EXTENSION_NAME} extension"
            )));
        }
        let mut xlib_xcb: Option<(SharedObject, PfnXGetXCBConnection)> = Option::None;
        if has_xlib_surface_extension {
            // (vulkan_xlib_xcb_library = NULL)
        } else if !has_xcb_surface_extension {
            return Err(Error::new(format!(
                "Installed Vulkan doesn't implement either the {VK_KHR_XCB_SURFACE_EXTENSION_NAME} extension or the {VK_KHR_XLIB_SURFACE_EXTENSION_NAME} extension"
            )));
        } else {
            let lib_x11_xcb_library_name = hints::get(hints::X11_XCB_LIBRARY)
                .filter(|h| !h.is_empty())
                .unwrap_or_else(|| DEFAULT_X11_XCB.to_owned());
            let library = SharedObject::load(&lib_x11_xcb_library_name)?;
            // SAFETY: XGetXCBConnection has this signature.
            let x_get_xcb_connection =
                unsafe { library.function::<PfnXGetXCBConnection>("XGetXCBConnection")? };
            xlib_xcb = Some((library, x_get_xcb_connection));
        }
        self.with_data(|d| {
            d.vulkan.loader_handle = Some(loader_handle);
            d.vulkan.vk_get_instance_proc_addr = Some(vk_get_instance_proc_addr);
            match xlib_xcb {
                Some((library, f)) => {
                    d.vulkan.vulkan_xlib_xcb_library = Some(library);
                    d.vulkan.vulkan_x_get_xcb_connection = Some(f);
                    d.vulkan.uses_xcb = true;
                }
                Option::None => {
                    d.vulkan.vulkan_xlib_xcb_library = Option::None;
                    d.vulkan.vulkan_x_get_xcb_connection = Option::None;
                    d.vulkan.uses_xcb = false;
                }
            }
        });
        Ok(())
    }

    /// Translation of `X11_Vulkan_UnloadLibrary()`.
    pub(crate) fn x11_vulkan_unload_library(&self) {
        let old = self.with_data(|d| {
            if d.vulkan.loader_handle.is_some() {
                // FIXME (upstream): vulkan_xlib_xcb_library is unloaded but
                // not cleared, so X11_Vulkan_GetInstanceExtensions() still
                // reports the XCB extension until the next load (`uses_xcb`
                // is kept for that).
                let xcb = d.vulkan.vulkan_xlib_xcb_library.take();
                d.vulkan.vulkan_x_get_xcb_connection = Option::None;
                let loader = d.vulkan.loader_handle.take();
                d.vulkan.vk_get_instance_proc_addr = Option::None;
                Some((xcb, loader))
            } else {
                Option::None
            }
        });
        // (dropping the libraries unloads them)
        drop(old);
    }

    /// The loader's `vkGetInstanceProcAddr` (`vulkan_config.vkGetInstanceProcAddr`).
    pub(crate) fn x11_vulkan_get_instance_proc_addr(&self) -> Option<usize> {
        self.with_data(|d| d.vulkan.vk_get_instance_proc_addr.map(|f| f as usize))
    }

    /// Translation of `X11_Vulkan_GetInstanceExtensions()`.
    pub(crate) fn x11_vulkan_get_instance_extensions(&self) -> Vec<&'static str> {
        if self.with_data(|d| d.vulkan.uses_xcb) {
            vec![
                VK_KHR_SURFACE_EXTENSION_NAME,
                VK_KHR_XCB_SURFACE_EXTENSION_NAME,
            ]
        } else {
            vec![
                VK_KHR_SURFACE_EXTENSION_NAME,
                VK_KHR_XLIB_SURFACE_EXTENSION_NAME,
            ]
        }
    }

    /// Translation of `X11_Vulkan_CreateSurface()`.
    pub(crate) fn x11_vulkan_create_surface(
        &self,
        window: WindowID,
        instance: usize,
        allocator: usize,
    ) -> Result<u64> {
        let (get, x_get_xcb_connection) = self.with_data(|d| {
            (
                d.vulkan.vk_get_instance_proc_addr,
                d.vulkan.vulkan_x_get_xcb_connection,
            )
        });
        let Some(vk_get_instance_proc_addr) = get else {
            return Err(Error::new("Vulkan is not loaded"));
        };
        let xwindow = with_x11_window(window, |_, data| data.xwindow)?;
        let mut surface: u64 = 0;
        if let Some(x_get_xcb_connection) = x_get_xcb_connection {
            // SAFETY: the function's type.
            let vk_create_xcb_surface_khr: Option<PfnVkCreateXcbSurfaceKHR> = unsafe {
                instance_function(vk_get_instance_proc_addr, instance, "vkCreateXcbSurfaceKHR")
            };
            let Some(vk_create_xcb_surface_khr) = vk_create_xcb_surface_khr else {
                return Err(Error::new(format!(
                    "{VK_KHR_XCB_SURFACE_EXTENSION_NAME} extension is not enabled in the Vulkan instance."
                )));
            };
            let create_info = VkXcbSurfaceCreateInfoKHR {
                s_type: VK_STRUCTURE_TYPE_XCB_SURFACE_CREATE_INFO_KHR,
                p_next: std::ptr::null(),
                flags: 0,
                // SAFETY: the display is open.
                connection: unsafe { x_get_xcb_connection(self.display) },
                window: xwindow as u32,
            };
            if create_info.connection.is_null() {
                return Err(Error::new("XGetXCBConnection failed"));
            }
            // SAFETY: the instance and allocator are the caller's; the
            // create info is valid.
            let result = unsafe {
                vk_create_xcb_surface_khr(
                    instance as *mut c_void,
                    &create_info,
                    allocator as *const c_void,
                    &mut surface,
                )
            };
            if result != VK_SUCCESS {
                return Err(Error::new(format!(
                    "vkCreateXcbSurfaceKHR failed: {}",
                    vulkan_get_result_string(result)
                )));
            }
        } else {
            // SAFETY: the function's type.
            let vk_create_xlib_surface_khr: Option<PfnVkCreateXlibSurfaceKHR> = unsafe {
                instance_function(
                    vk_get_instance_proc_addr,
                    instance,
                    "vkCreateXlibSurfaceKHR",
                )
            };
            let Some(vk_create_xlib_surface_khr) = vk_create_xlib_surface_khr else {
                return Err(Error::new(format!(
                    "{VK_KHR_XLIB_SURFACE_EXTENSION_NAME} extension is not enabled in the Vulkan instance."
                )));
            };
            let create_info = VkXlibSurfaceCreateInfoKHR {
                s_type: VK_STRUCTURE_TYPE_XLIB_SURFACE_CREATE_INFO_KHR,
                p_next: std::ptr::null(),
                flags: 0,
                dpy: self.display,
                window: xwindow as u32 as Window,
            };
            // SAFETY: as above.
            let result = unsafe {
                vk_create_xlib_surface_khr(
                    instance as *mut c_void,
                    &create_info,
                    allocator as *const c_void,
                    &mut surface,
                )
            };
            if result != VK_SUCCESS {
                return Err(Error::new(format!(
                    "vkCreateXlibSurfaceKHR failed: {}",
                    vulkan_get_result_string(result)
                )));
            }
        }

        Ok(surface) // success!
    }

    /// Translation of `X11_Vulkan_DestroySurface()` (and
    /// `SDL_Vulkan_DestroySurface_Internal()`).
    pub(crate) fn x11_vulkan_destroy_surface(
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
                    );
                }
            }
        }
    }

    /// Translation of `X11_Vulkan_GetPresentationSupport()`.
    pub(crate) fn x11_vulkan_get_presentation_support(
        &self,
        instance: usize,
        physical_device: usize,
        queue_family_index: u32,
    ) -> bool {
        let (get, x_get_xcb_connection) = self.with_data(|d| {
            (
                d.vulkan.vk_get_instance_proc_addr,
                d.vulkan.vulkan_x_get_xcb_connection,
            )
        });
        let Some(vk_get_instance_proc_addr) = get else {
            // (SDL_SetError("Vulkan is not loaded"))
            return false;
        };

        let display = self.display;
        let visualid: VisualID = match hints::get(hints::VIDEO_X11_WINDOW_VISUALID) {
            Some(forced_visual_id) => {
                crate::stdlib::string::strtol(&forced_visual_id, 0).0 as c_ulong
            }
            // SAFETY: the display is open.
            Option::None => unsafe {
                (self.x.XVisualIDFromVisual)(DefaultVisual(display, DefaultScreen(display)))
            },
        };

        if let Some(x_get_xcb_connection) = x_get_xcb_connection {
            // SAFETY: the function's type.
            let f: Option<PfnVkGetPhysicalDeviceXcbPresentationSupportKHR> = unsafe {
                instance_function(
                    vk_get_instance_proc_addr,
                    instance,
                    "vkGetPhysicalDeviceXcbPresentationSupportKHR",
                )
            };

            let Some(f) = f else {
                // (SDL_SetError: the XCB surface extension is not enabled)
                return false;
            };

            // SAFETY: the caller's physical device; the display is open.
            unsafe {
                f(
                    physical_device as *mut c_void,
                    queue_family_index,
                    x_get_xcb_connection(display),
                    visualid as u32,
                ) != 0
            }
        } else {
            // SAFETY: the function's type.
            let f: Option<PfnVkGetPhysicalDeviceXlibPresentationSupportKHR> = unsafe {
                instance_function(
                    vk_get_instance_proc_addr,
                    instance,
                    "vkGetPhysicalDeviceXlibPresentationSupportKHR",
                )
            };

            let Some(f) = f else {
                // (SDL_SetError: the Xlib surface extension is not enabled)
                return false;
            };

            // SAFETY: as above.
            unsafe {
                f(
                    physical_device as *mut c_void,
                    queue_family_index,
                    display,
                    visualid,
                ) != 0
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_strings() {
        assert_eq!(vulkan_get_result_string(0), "VK_SUCCESS");
        assert_eq!(vulkan_get_result_string(-9), "VK_ERROR_INCOMPATIBLE_DRIVER");
        assert_eq!(vulkan_get_result_string(-424242), "VK_ERROR_<Unknown>");
        assert_eq!(vulkan_get_result_string(424242), "VK_<Unknown>");
    }
}
