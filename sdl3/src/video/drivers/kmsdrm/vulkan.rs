// Rust translation of src/video/kmsdrm/SDL_kmsdrmvulkan.c and
// SDL_kmsdrmvulkan.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * @author Manuel Alfayate Corchere <redwindwanderer@gmail.com>.
 * Based on Jacob Lifshay's SDL_x11vulkan.c.
 */

//! Vulkan on the KMS/DRM driver's windows: the Vulkan loader, loaded at
//! run time, which must have `VK_KHR_surface` and `VK_KHR_display`, and
//! window surfaces on a display plane (`vkCreateDisplayPlaneSurfaceKHR()`,
//! "x-less VK"): the first integrated or discrete GPU, the display of the
//! window's display index, a mode of the window's size (one of the
//! display's, the highest refresh rate first, or one made with
//! `vkCreateDisplayModeKHR()`), and a plane that can show it.
//!
//! The Vulkan driver drives the display itself (through its own DRM file
//! descriptor), so a Vulkan window has no GBM or EGL surface, and while
//! there is one the driver is in Vulkan mode (`vulkan_mode`, in
//! [`video`](super::video)): mode sets and window resizes do nothing.

use std::ffi::{c_char, c_void, CStr};
use std::sync::Mutex;

use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::hints;
use crate::loadso::SharedObject;
use crate::video::core::with_window;
use crate::video::display;
use crate::video::vulkan_utils::{
    instance_function, vulkan_create_instance_extensions_list, vulkan_destroy_surface_internal,
    vulkan_get_result_string, PfnVkEnumerateInstanceExtensionProperties, PfnVkGetInstanceProcAddr,
    VkResult, VK_KHR_SURFACE_EXTENSION_NAME, VK_SUCCESS,
};
use crate::video::window::display_for_window;

#[cfg(target_os = "openbsd")]
const DEFAULT_VULKAN: &str = "libvulkan.so";
#[cfg(not(target_os = "openbsd"))]
const DEFAULT_VULKAN: &str = "libvulkan.so.1";

// (SDL_ELF_NOTE_DLOPEN "kmsdrm-vulkan", "Support for Vulkan on KMSDRM",
// suggested, DEFAULT_VULKAN: upstream's ELF note for packagers; none here)

/// `VK_KHR_DISPLAY_EXTENSION_NAME`.
pub(crate) const VK_KHR_DISPLAY_EXTENSION_NAME: &str = "VK_KHR_display";

/// `VK_STRUCTURE_TYPE_DISPLAY_MODE_CREATE_INFO_KHR`.
const VK_STRUCTURE_TYPE_DISPLAY_MODE_CREATE_INFO_KHR: i32 = 1000002000;
/// `VK_STRUCTURE_TYPE_DISPLAY_SURFACE_CREATE_INFO_KHR`.
const VK_STRUCTURE_TYPE_DISPLAY_SURFACE_CREATE_INFO_KHR: i32 = 1000002001;
/// `VK_DISPLAY_PLANE_ALPHA_OPAQUE_BIT_KHR`.
const VK_DISPLAY_PLANE_ALPHA_OPAQUE_BIT_KHR: u32 = 0x00000001;
/// `VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR`.
const VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR: u32 = 0x00000001;
/// `VK_NULL_HANDLE` (of a non-dispatchable handle).
const VK_NULL_HANDLE: u64 = 0;

/// `VkPhysicalDevice` (a dispatchable handle).
type VkPhysicalDevice = *mut c_void;
/// `VkDisplayKHR` (non-dispatchable: 64 bits on every platform).
type VkDisplayKHR = u64;
/// `VkDisplayModeKHR` (non-dispatchable).
type VkDisplayModeKHR = u64;

/// `VkExtent2D`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkExtent2D {
    width: u32,
    height: u32,
}

/// `VkOffset2D`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkOffset2D {
    x: i32,
    y: i32,
}

/// `VkPhysicalDeviceProperties`, its `limits` and `sparseProperties`
/// (which aren't read) as their bytes.
#[repr(C)]
struct VkPhysicalDeviceProperties {
    api_version: u32,
    driver_version: u32,
    vendor_id: u32,
    device_id: u32,
    device_type: i32,
    device_name: [c_char; 256],
    pipeline_cache_uuid: [u8; 16],
    /// `VkPhysicalDeviceLimits limits` (504 bytes) and
    /// `VkPhysicalDeviceSparseProperties sparseProperties` (20), padded
    limits_and_sparse_properties: [u64; 66],
}

/// `VkDisplayPropertiesKHR`.
#[repr(C)]
#[derive(Clone, Copy)]
struct VkDisplayPropertiesKHR {
    display: VkDisplayKHR,
    display_name: *const c_char,
    physical_dimensions: VkExtent2D,
    physical_resolution: VkExtent2D,
    supported_transforms: u32,
    plane_reorder_possible: u32,
    persistent_content: u32,
}

/// `VkDisplayModeParametersKHR`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkDisplayModeParametersKHR {
    visible_region: VkExtent2D,
    refresh_rate: u32,
}

/// `VkDisplayModePropertiesKHR`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkDisplayModePropertiesKHR {
    display_mode: VkDisplayModeKHR,
    parameters: VkDisplayModeParametersKHR,
}

/// `VkDisplayPlanePropertiesKHR`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkDisplayPlanePropertiesKHR {
    current_display: VkDisplayKHR,
    current_stack_index: u32,
}

/// `VkDisplayPlaneCapabilitiesKHR`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VkDisplayPlaneCapabilitiesKHR {
    supported_alpha: u32,
    min_src_position: VkOffset2D,
    max_src_position: VkOffset2D,
    min_src_extent: VkExtent2D,
    max_src_extent: VkExtent2D,
    min_dst_position: VkOffset2D,
    max_dst_position: VkOffset2D,
    min_dst_extent: VkExtent2D,
    max_dst_extent: VkExtent2D,
}

/// `VkDisplayModeCreateInfoKHR`.
#[repr(C)]
struct VkDisplayModeCreateInfoKHR {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    parameters: VkDisplayModeParametersKHR,
}

/// `VkDisplaySurfaceCreateInfoKHR`.
#[repr(C)]
struct VkDisplaySurfaceCreateInfoKHR {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    display_mode: VkDisplayModeKHR,
    plane_index: u32,
    plane_stack_index: u32,
    transform: u32,
    global_alpha: f32,
    alpha_mode: u32,
    image_extent: VkExtent2D,
}

type PfnVkCreateDisplayPlaneSurfaceKHR = unsafe extern "system" fn(
    *mut c_void,
    *const VkDisplaySurfaceCreateInfoKHR,
    *const c_void,
    *mut u64,
) -> VkResult;
type PfnVkEnumeratePhysicalDevices =
    unsafe extern "system" fn(*mut c_void, *mut u32, *mut VkPhysicalDevice) -> VkResult;
type PfnVkGetPhysicalDeviceProperties =
    unsafe extern "system" fn(VkPhysicalDevice, *mut VkPhysicalDeviceProperties);
type PfnVkGetPhysicalDeviceDisplayPropertiesKHR =
    unsafe extern "system" fn(VkPhysicalDevice, *mut u32, *mut VkDisplayPropertiesKHR) -> VkResult;
type PfnVkGetDisplayModePropertiesKHR = unsafe extern "system" fn(
    VkPhysicalDevice,
    VkDisplayKHR,
    *mut u32,
    *mut VkDisplayModePropertiesKHR,
) -> VkResult;
type PfnVkGetPhysicalDeviceDisplayPlanePropertiesKHR = unsafe extern "system" fn(
    VkPhysicalDevice,
    *mut u32,
    *mut VkDisplayPlanePropertiesKHR,
) -> VkResult;
type PfnVkGetDisplayPlaneSupportedDisplaysKHR =
    unsafe extern "system" fn(VkPhysicalDevice, u32, *mut u32, *mut VkDisplayKHR) -> VkResult;
type PfnVkGetDisplayPlaneCapabilitiesKHR = unsafe extern "system" fn(
    VkPhysicalDevice,
    VkDisplayModeKHR,
    u32,
    *mut VkDisplayPlaneCapabilitiesKHR,
) -> VkResult;
type PfnVkCreateDisplayModeKHR = unsafe extern "system" fn(
    VkPhysicalDevice,
    VkDisplayKHR,
    *const VkDisplayModeCreateInfoKHR,
    *const c_void,
    *mut VkDisplayModeKHR,
) -> VkResult;

/// The loader (`_this->vulkan_config`: `loader_handle`, `loader_path`,
/// `vkGetInstanceProcAddr` and `vkEnumerateInstanceExtensionProperties`).
#[derive(Default)]
pub(crate) struct VulkanConfig {
    loader_handle: Option<SharedObject>,
    loader_path: Option<String>,
    vk_get_instance_proc_addr: Option<PfnVkGetInstanceProcAddr>,
    vk_enumerate_instance_extension_properties: Option<PfnVkEnumerateInstanceExtensionProperties>,
}

fn lock(config: &Mutex<VulkanConfig>) -> std::sync::MutexGuard<'_, VulkanConfig> {
    config.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `KMSDRM_Vulkan_LoadLibrary()`.
pub(crate) fn kmsdrm_vulkan_load_library(
    config: &Mutex<VulkanConfig>,
    path: Option<&str>,
) -> Result<()> {
    let mut has_surface_extension = false;
    let mut has_display_extension = false;

    if lock(config).loader_handle.is_some() {
        return Err(Error::new("Vulkan already loaded"));
    }

    // Load the Vulkan library
    let hint = match path {
        Some(_) => None,
        None => hints::get(hints::VULKAN_LIBRARY),
    };
    let path = path.or(hint.as_deref()).unwrap_or(DEFAULT_VULKAN);

    let loader_handle = SharedObject::load(path)?;

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
        } else if name == VK_KHR_DISPLAY_EXTENSION_NAME {
            has_display_extension = true;
        }
    }

    if !has_surface_extension {
        return Err(Error::new(format!(
            "Installed Vulkan doesn't implement the {VK_KHR_SURFACE_EXTENSION_NAME} extension"
        )));
    } else if !has_display_extension {
        return Err(Error::new(format!(
            "Installed Vulkan doesn't implement the {VK_KHR_DISPLAY_EXTENSION_NAME} extension"
        )));
    }

    let mut config = lock(config);
    config.loader_handle = Some(loader_handle);
    config.loader_path = Some(path.to_owned());
    config.vk_get_instance_proc_addr = Some(vk_get_instance_proc_addr);
    config.vk_enumerate_instance_extension_properties =
        Some(vk_enumerate_instance_extension_properties);
    Ok(())
}

/// Translation of `KMSDRM_Vulkan_UnloadLibrary()`.
///
/// Note (upstream): only the loader handle is cleared there, the function
/// pointers of the unloaded library kept; here they go with it.
pub(crate) fn kmsdrm_vulkan_unload_library(config: &Mutex<VulkanConfig>) {
    let old = std::mem::take(&mut *lock(config));
    // (dropping the handle unloads the library)
    drop(old);
}

/// The path `KMSDRM_Vulkan_LoadLibrary()` copied to
/// `vulkan_config.loader_path`.
pub(crate) fn kmsdrm_vulkan_loader_path(config: &Mutex<VulkanConfig>) -> Option<String> {
    lock(config).loader_path.clone()
}

/// The loader's `vkGetInstanceProcAddr` (`vulkan_config.vkGetInstanceProcAddr`).
pub(crate) fn kmsdrm_vulkan_get_instance_proc_addr(config: &Mutex<VulkanConfig>) -> Option<usize> {
    lock(config).vk_get_instance_proc_addr.map(|f| f as usize)
}

/*********************************************************************/
// Here we can put whatever Vulkan extensions we want to be enabled
// at instance creation, which is done in the programs, not in SDL.
// So: programs call SDL_Vulkan_GetInstanceExtensions() and here
// we put the extensions specific to this backend so the programs
// get a list with the extension we want, so they can include that
// list in the ppEnabledExtensionNames and EnabledExtensionCount
// members of the VkInstanceCreateInfo struct passed to
// vkCreateInstance().
/*********************************************************************/
/// Translation of `KMSDRM_Vulkan_GetInstanceExtensions()`.
pub(crate) fn kmsdrm_vulkan_get_instance_extensions() -> Vec<&'static str> {
    vec![VK_KHR_SURFACE_EXTENSION_NAME, VK_KHR_DISPLAY_EXTENSION_NAME]
}

/// Translation of `display_mode_comparator()`: the larger refresh rate
/// first.
fn display_mode_comparator(
    a: &VkDisplayModePropertiesKHR,
    b: &VkDisplayModePropertiesKHR,
) -> std::cmp::Ordering {
    b.parameters.refresh_rate.cmp(&a.parameters.refresh_rate)
}

/// A Vulkan function of the instance, through the loader's
/// `vkGetInstanceProcAddr` (`NULL`: `None`).
macro_rules! get_instance_function {
    ($get:expr, $instance:expr, $name:literal) => {
        // SAFETY: the loader's vkGetInstanceProcAddr and the caller's
        // instance; the function's type is the one it's stored as.
        unsafe { instance_function($get, $instance, $name) }
    };
}

/// The values of an array a Vulkan function fills in two calls, the count
/// first (`vkGet...(..., &count, NULL)`, then the array).
fn enumerate<T: Copy>(zero: T, mut get: impl FnMut(*mut u32, *mut T)) -> Vec<T> {
    let mut count: u32 = 0;
    get(&mut count, std::ptr::null_mut());
    if count == 0 {
        return Vec::new();
    }
    let mut values = vec![zero; count as usize];
    get(&mut count, values.as_mut_ptr());
    values.truncate(count as usize);
    values
}

/***********************************************************************/
// First thing to know is that we don't call vkCreateInstance() here.
// Instead, programs using SDL and Vulkan create their Vulkan instance
// and we get it here, ready to use.
// Extensions specific for this platform are activated in
// KMSDRM_Vulkan_GetInstanceExtensions(), like we do with
// VK_KHR_DISPLAY_EXTENSION_NAME, which is what we need for x-less VK.
/***********************************************************************/
/// Translation of `KMSDRM_Vulkan_CreateSurface()`.
pub(crate) fn kmsdrm_vulkan_create_surface(
    config: &Mutex<VulkanConfig>,
    window: WindowID,
    instance: usize,
    allocator: usize,
) -> Result<u64> {
    let mut gpu: VkPhysicalDevice = std::ptr::null_mut();
    let mut plane: u32 = u32::MAX;

    let mut display_mode: VkDisplayModeKHR = 0;
    let mut display_mode_props = VkDisplayModePropertiesKHR::default();
    // Prefer a plane that supports per-pixel alpha.
    let alpha_mode = VK_DISPLAY_PLANE_ALPHA_OPAQUE_BIT_KHR;

    let mut valid_gpu = false;
    let mut mode_found = false;

    // Get the display index from the display being used by the window.
    // Note (upstream): SDL_GetDisplayIndex()'s -1 (no display) indexes
    // the display properties at -1 there; here it is out of range (below).
    let display_index = display_for_window(window)
        .and_then(display::display_index)
        .ok();
    let (window_w, window_h, refresh_rate) = with_window(window, |w| {
        (w.core.w, w.core.h, w.current_fullscreen_mode.refresh_rate)
    })?;

    // Get the function pointers for the functions we will use.
    let (loaded, get) = {
        let config = lock(config);
        (
            config.loader_handle.is_some(),
            config.vk_get_instance_proc_addr,
        )
    };
    // Note (upstream): the functions are looked up before the check that
    // Vulkan is loaded, through a NULL (or the unloaded library's)
    // vkGetInstanceProcAddr when it isn't.
    let (true, Some(get)) = (loaded, get) else {
        return Err(Error::new("Vulkan is not loaded"));
    };
    let instance = instance as *mut c_void;

    let vk_create_display_plane_surface_khr: Option<PfnVkCreateDisplayPlaneSurfaceKHR> =
        get_instance_function!(get, instance, c"vkCreateDisplayPlaneSurfaceKHR");
    let vk_enumerate_physical_devices: Option<PfnVkEnumeratePhysicalDevices> =
        get_instance_function!(get, instance, c"vkEnumeratePhysicalDevices");
    let vk_get_physical_device_properties: Option<PfnVkGetPhysicalDeviceProperties> =
        get_instance_function!(get, instance, c"vkGetPhysicalDeviceProperties");
    let vk_get_physical_device_display_properties_khr: Option<
        PfnVkGetPhysicalDeviceDisplayPropertiesKHR,
    > = get_instance_function!(get, instance, c"vkGetPhysicalDeviceDisplayPropertiesKHR");
    let vk_get_display_mode_properties_khr: Option<PfnVkGetDisplayModePropertiesKHR> =
        get_instance_function!(get, instance, c"vkGetDisplayModePropertiesKHR");
    let vk_get_physical_device_display_plane_properties_khr: Option<
        PfnVkGetPhysicalDeviceDisplayPlanePropertiesKHR,
    > = get_instance_function!(
        get,
        instance,
        c"vkGetPhysicalDeviceDisplayPlanePropertiesKHR"
    );
    let vk_get_display_plane_supported_displays_khr: Option<
        PfnVkGetDisplayPlaneSupportedDisplaysKHR,
    > = get_instance_function!(get, instance, c"vkGetDisplayPlaneSupportedDisplaysKHR");
    let vk_get_display_plane_capabilities_khr: Option<PfnVkGetDisplayPlaneCapabilitiesKHR> =
        get_instance_function!(get, instance, c"vkGetDisplayPlaneCapabilitiesKHR");
    let vk_create_display_mode_khr: Option<PfnVkCreateDisplayModeKHR> =
        get_instance_function!(get, instance, c"vkCreateDisplayModeKHR");

    /*************************************/
    // Block for vulkan surface creation
    /*************************************/

    /****************************************************************/
    // If we got vkCreateDisplayPlaneSurfaceKHR() pointer, it means
    // that the VK_KHR_Display extension is active on the instance.
    // That's the central extension we need for x-less VK!
    /****************************************************************/
    // (with it, the other functions of VK_KHR_display and Vulkan 1.0's are
    // there too: upstream calls them unchecked)
    let (
        Some(vk_create_display_plane_surface_khr),
        Some(vk_enumerate_physical_devices),
        Some(vk_get_physical_device_properties),
        Some(vk_get_physical_device_display_properties_khr),
        Some(vk_get_display_mode_properties_khr),
        Some(vk_get_physical_device_display_plane_properties_khr),
        Some(vk_get_display_plane_supported_displays_khr),
        Some(vk_get_display_plane_capabilities_khr),
        Some(vk_create_display_mode_khr),
    ) = (
        vk_create_display_plane_surface_khr,
        vk_enumerate_physical_devices,
        vk_get_physical_device_properties,
        vk_get_physical_device_display_properties_khr,
        vk_get_display_mode_properties_khr,
        vk_get_physical_device_display_plane_properties_khr,
        vk_get_display_plane_supported_displays_khr,
        vk_get_display_plane_capabilities_khr,
        vk_create_display_mode_khr,
    )
    else {
        return Err(Error::new(format!(
            "{VK_KHR_DISPLAY_EXTENSION_NAME} extension is not enabled in the Vulkan instance."
        )));
    };

    /* A GPU (or physical_device, in vkcube terms) is a physical GPU.
    A machine with more than one video output doesn't need to have more than one GPU,
    like the Pi4 which has 1 GPU and 2 video outputs.
    Just in case, we test that the GPU we choose is Vulkan-capable.
    If there are new reports about VK init failures, hardcode
    gpu = physical_devices[0], instead of probing, and go with that.
    */

    // Get the physical device count.
    // Get the physical devices.
    // SAFETY: the instance's function; the count and the array it fills.
    let physical_devices = enumerate(std::ptr::null_mut(), |count, devices| unsafe {
        vk_enumerate_physical_devices(instance, count, devices);
    });

    if physical_devices.is_empty() {
        return Err(Error::new("Vulkan can't find physical devices (gpus)."));
    }

    // Iterate on the physical devices.
    for &physical_device in &physical_devices {
        // Get the physical device properties.
        // SAFETY: an all-zero VkPhysicalDeviceProperties is valid (numbers
        // and arrays); the instance's function fills it in.
        let device_props = unsafe {
            let mut device_props: VkPhysicalDeviceProperties = std::mem::zeroed();
            vk_get_physical_device_properties(physical_device, &mut device_props);
            device_props
        };

        // Is this device a real GPU that supports API version 1 at least?
        if device_props.api_version >= 1
            && (device_props.device_type == 1 || device_props.device_type == 2)
        {
            gpu = physical_device;
            valid_gpu = true;
            break;
        }
    }

    if !valid_gpu {
        return Err(Error::new(
            "Vulkan can't find a valid physical device (gpu).",
        ));
    }

    /* A display is a video output. 1 GPU can have N displays.
    Vulkan only counts the connected displays.
    Get the display count of the GPU. */
    // Get the props of the displays of the physical device.
    let zero_display = VkDisplayPropertiesKHR {
        display: VK_NULL_HANDLE,
        display_name: std::ptr::null(),
        physical_dimensions: VkExtent2D::default(),
        physical_resolution: VkExtent2D::default(),
        supported_transforms: 0,
        plane_reorder_possible: 0,
        persistent_content: 0,
    };
    // SAFETY: the GPU's function; the count and the array it fills.
    let display_props = enumerate(zero_display, |count, props| unsafe {
        vk_get_physical_device_display_properties_khr(gpu, count, props);
    });
    if display_props.is_empty() {
        return Err(Error::new("Vulkan can't find any displays."));
    }

    // Get the chosen display based on the display index.
    // Note (upstream): the index isn't checked against the display count
    // there (SDL's displays aren't the GPU's); here one out of range is the
    // error of a GPU without displays.
    let Some(chosen) = display_index.and_then(|i| display_props.get(i)) else {
        return Err(Error::new("Vulkan can't find any displays."));
    };
    let display = chosen.display;

    // Get the list of the display videomodes.
    // SAFETY: the GPU's function, its display; the count and the array it
    // fills.
    let mut mode_props = enumerate(
        VkDisplayModePropertiesKHR::default(),
        |count, props| unsafe {
            vk_get_display_mode_properties_khr(gpu, display, count, props);
        },
    );

    if mode_props.is_empty() {
        let name = if chosen.display_name.is_null() {
            "(null)".into()
        } else {
            // SAFETY: the driver's NUL-terminated name, which lives as long
            // as the instance.
            unsafe { CStr::from_ptr(chosen.display_name) }.to_string_lossy()
        };
        // (upstream's display number is always 0)
        return Err(Error::new(format!(
            "Vulkan can't find any video modes for display {} ({})",
            0, name
        )));
    }

    /* Sort the modes so that larger refresh rates come first. */
    // (a stable sort, where SDL_qsort() isn't: modes of the same refresh
    // rate keep the driver's order)
    mode_props.sort_by(display_mode_comparator);

    /* Get a video mode equal to the window size among the predefined ones,
    if possible.
    REMEMBER: We have to get a small enough videomode for the window size,
    because videomode determines how big the scanout region is and we can't
    scanout a region bigger than the window (we would be reading past the
    buffer, and Vulkan would give us a confusing VK_ERROR_SURFACE_LOST_KHR). */
    for m in &mode_props {
        if m.parameters.visible_region.width as i32 == window_w
            && m.parameters.visible_region.height as i32 == window_h
        {
            display_mode_props = *m;
            mode_found = true;
            break;
        }
    }

    if mode_found
        && display_mode_props.parameters.visible_region.width > 0
        && display_mode_props.parameters.visible_region.height > 0
    {
        // Found a suitable mode among the predefined ones. Use that.
        display_mode = display_mode_props.display_mode;
    } else {
        /* Couldn't find a suitable mode among the predefined ones, so try to create our own.
        This won't work for some video chips atm (like Pi's VideoCore) so these are limited
        to supported resolutions. Don't try to use "closest" resolutions either, because
        those are often bigger than the window size, thus causing out-of-bunds scanout. */
        let new_mode_parameters = VkDisplayModeParametersKHR {
            visible_region: VkExtent2D {
                width: window_w as u32,
                height: window_h as u32,
            },
            /* SDL (and DRM, if we look at drmModeModeInfo vrefresh) uses plain integer Hz for
            display mode refresh rate, but Vulkan expects higher precision. */
            refresh_rate: (refresh_rate * 1000.0) as u32,
        };

        let display_mode_create_info = VkDisplayModeCreateInfoKHR {
            s_type: VK_STRUCTURE_TYPE_DISPLAY_MODE_CREATE_INFO_KHR,
            p_next: std::ptr::null(),
            flags: 0,
            parameters: new_mode_parameters,
        };
        // SAFETY: the GPU's function, its display; the create info is
        // valid; the mode is an out-parameter.
        let result = unsafe {
            vk_create_display_mode_khr(
                gpu,
                display,
                &display_mode_create_info,
                std::ptr::null(),
                &mut display_mode,
            )
        };
        if result != VK_SUCCESS {
            return Err(Error::new("Vulkan couldn't find a predefined mode for that window size and couldn't create a suitable mode."));
        }
    }

    // Just in case we get here without a display_mode.
    if display_mode == 0 {
        return Err(Error::new("Vulkan couldn't get a display mode."));
    }

    // Get the list of the physical device planes.
    // SAFETY: the GPU's function; the count and the array it fills.
    let plane_props = enumerate(
        VkDisplayPlanePropertiesKHR::default(),
        |count, props| unsafe {
            vk_get_physical_device_display_plane_properties_khr(gpu, count, props);
        },
    );
    if plane_props.is_empty() {
        return Err(Error::new("Vulkan can't find any planes."));
    }

    /* Iterate on the list of planes of the physical device
    to find a plane that matches these criteria:
    -It must be compatible with the chosen display + mode.
    -It isn't currently bound to another display.
    -It supports per-pixel alpha, if possible. */
    for (i, props) in plane_props.iter().enumerate() {
        let i = i as u32;

        // See if the plane is compatible with the current display.
        // Get the list of displays supported by this plane.
        // SAFETY: the GPU's function, its plane; the count and the array it
        // fills.
        let supported_displays = enumerate(VK_NULL_HANDLE, |count, displays| unsafe {
            vk_get_display_plane_supported_displays_khr(gpu, i, count, displays);
        });
        if supported_displays.is_empty() {
            // This plane doesn't support any displays. Continue to the next plane.
            continue;
        }

        /* The plane must be bound to the chosen display, or not in use.
        If none of these is true, iterate to another plane. */
        if !((props.current_display == display) || (props.current_display == VK_NULL_HANDLE)) {
            continue;
        }

        /* Iterate the list of displays supported by this plane
        in order to find out if the chosen display is among them. */
        let plane_supports_display = supported_displays.contains(&display);

        // If the display is not supported by this plane, iterate to the next plane.
        if !plane_supports_display {
            continue;
        }

        // Want a plane that supports the alpha mode we have chosen.
        let mut plane_caps = VkDisplayPlaneCapabilitiesKHR::default();
        // SAFETY: the GPU's function, its mode and plane; an out-parameter.
        unsafe { vk_get_display_plane_capabilities_khr(gpu, display_mode, i, &mut plane_caps) };
        // (upstream compares the supported modes' bits with the one mode:
        // a plane that supports others besides it doesn't match)
        if plane_caps.supported_alpha == alpha_mode {
            // Yep, this plane is alright.
            plane = i;
            break;
        }
    }

    // If we couldn't find an appropriate plane, error out.
    if plane == u32::MAX {
        return Err(Error::new("Vulkan couldn't find an appropriate plane."));
    }

    /********************************************/
    // Let's finally create the Vulkan surface!
    /********************************************/

    let image_size = VkExtent2D {
        width: window_w as u32,
        height: window_h as u32,
    };

    let display_plane_surface_create_info = VkDisplaySurfaceCreateInfoKHR {
        s_type: VK_STRUCTURE_TYPE_DISPLAY_SURFACE_CREATE_INFO_KHR,
        p_next: std::ptr::null(),
        flags: 0,
        display_mode,
        plane_index: plane,
        plane_stack_index: 0,
        transform: VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR,
        global_alpha: 0.0,
        alpha_mode,
        image_extent: image_size,
    };
    let mut surface: u64 = 0;
    // SAFETY: the caller's instance and allocator; the create info is
    // valid; the surface is an out-parameter.
    let result = unsafe {
        vk_create_display_plane_surface_khr(
            instance,
            &display_plane_surface_create_info,
            allocator as *const c_void,
            &mut surface,
        )
    };
    if result != VK_SUCCESS {
        return Err(Error::new(format!(
            "vkCreateDisplayPlaneSurfaceKHR failed: {}",
            vulkan_get_result_string(result)
        )));
    }

    Ok(surface) // success!

    // (clean: the lists are dropped)
}

/// Translation of `KMSDRM_Vulkan_DestroySurface()`.
pub(crate) fn kmsdrm_vulkan_destroy_surface(
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
        // SAFETY: the caller's instance, surface and allocator, which the
        // video core checked aren't null.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_loader_and_extensions() {
        #[cfg(target_os = "openbsd")]
        assert_eq!(DEFAULT_VULKAN, "libvulkan.so");
        #[cfg(not(target_os = "openbsd"))]
        assert_eq!(DEFAULT_VULKAN, "libvulkan.so.1");
        assert_eq!(
            kmsdrm_vulkan_get_instance_extensions(),
            ["VK_KHR_surface", "VK_KHR_display"]
        );
    }

    /// vulkan_core.h's sizes and offsets on 64-bit Linux.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn layouts() {
        use std::mem::{offset_of, size_of};
        assert_eq!(size_of::<VkPhysicalDeviceProperties>(), 824);
        assert_eq!(offset_of!(VkPhysicalDeviceProperties, device_type), 16);
        assert_eq!(
            offset_of!(VkPhysicalDeviceProperties, limits_and_sparse_properties),
            296
        );
        assert_eq!(size_of::<VkDisplayPropertiesKHR>(), 48);
        assert_eq!(offset_of!(VkDisplayPropertiesKHR, persistent_content), 40);
        assert_eq!(size_of::<VkDisplayModeParametersKHR>(), 12);
        assert_eq!(size_of::<VkDisplayModePropertiesKHR>(), 24);
        assert_eq!(size_of::<VkDisplayPlanePropertiesKHR>(), 16);
        assert_eq!(size_of::<VkDisplayPlaneCapabilitiesKHR>(), 68);
        assert_eq!(size_of::<VkDisplayModeCreateInfoKHR>(), 32);
        assert_eq!(offset_of!(VkDisplayModeCreateInfoKHR, parameters), 20);
        assert_eq!(size_of::<VkDisplaySurfaceCreateInfoKHR>(), 64);
        assert_eq!(offset_of!(VkDisplaySurfaceCreateInfoKHR, display_mode), 24);
        assert_eq!(offset_of!(VkDisplaySurfaceCreateInfoKHR, image_extent), 52);
    }

    #[test]
    fn modes_by_refresh_rate() {
        let mode = |handle, w, h, refresh_rate| VkDisplayModePropertiesKHR {
            display_mode: handle,
            parameters: VkDisplayModeParametersKHR {
                visible_region: VkExtent2D {
                    width: w,
                    height: h,
                },
                refresh_rate,
            },
        };
        let mut modes = [
            mode(1, 640, 480, 59940),
            mode(2, 1024, 768, 60000),
            mode(3, 640, 480, 75000),
            mode(4, 800, 600, 60000),
        ];
        modes.sort_by(display_mode_comparator);
        let handles: Vec<u64> = modes.iter().map(|m| m.display_mode).collect();
        assert_eq!(handles, [3, 2, 4, 1]);
    }
}
