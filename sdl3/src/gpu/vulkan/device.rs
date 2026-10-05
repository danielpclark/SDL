// Rust translation of the device instantiation of
// src/gpu/vulkan/SDL_gpu_vulkan.c from Simple DirectMedia Layer: the
// extension and feature checks, the instance, the choice of the physical
// device, the logical device, and VULKAN_PrepareDriver(),
// VULKAN_CreateDevice() and VULKAN_DestroyDevice().
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Device instantiation. Upstream fills a zeroed `VulkanRenderer` step by
//! step (`VULKAN_INTERNAL_PrepareVulkan()` is shared by `PrepareDriver`,
//! which destroys what it made, and `CreateDevice`); here the steps fill a
//! [`PreparedVulkan`], which `CreateDevice` turns into the renderer once
//! the logical device exists.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Mutex, RwLock};

use super::memory::MemoryAllocator;
use super::resources::PendingDestroys;
use super::tables::vk_error_messages;
use super::vkfuncs::{DeviceFunctions, GlobalFunctions, InstanceFunctions};
use super::{set_string_error, vulkan_error, VulkanRenderer};
use crate::error::{Error, Result};
use crate::gpu::sysgpu::UNIFORM_BUFFER_SIZE;
use crate::gpu::{
    VulkanFeatureStructure, VulkanOptions, PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_FEATURE_ANISOTROPY_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_FEATURE_CLIP_DISTANCE_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_FEATURE_DEPTH_CLAMPING_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_FEATURE_INDIRECT_DRAW_FIRST_INSTANCE_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_PREFERLOWPOWER_BOOLEAN, PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_VERBOSE_BOOLEAN, PROP_GPU_DEVICE_CREATE_VULKAN_OPTIONS_POINTER,
    PROP_GPU_DEVICE_CREATE_VULKAN_REQUIRE_HARDWARE_ACCELERATION_BOOLEAN,
    PROP_GPU_DEVICE_DRIVER_INFO_STRING, PROP_GPU_DEVICE_DRIVER_NAME_STRING,
    PROP_GPU_DEVICE_DRIVER_VERSION_STRING, PROP_GPU_DEVICE_NAME_STRING,
};
use crate::log::Category;
use crate::properties::Properties;
use crate::thread::ReentrantMutex;
use crate::video::sysvideo::VideoDriver;
use crate::video::vk::*;

/// `VK_LAYER_KHRONOS_validation`
const VALIDATION_LAYER_NAME: &CStr = c"VK_LAYER_KHRONOS_validation";
const VK_EXT_DEBUG_UTILS_EXTENSION_NAME: &CStr = c"VK_EXT_debug_utils";
const VK_EXT_SWAPCHAIN_COLOR_SPACE_EXTENSION_NAME: &CStr = c"VK_EXT_swapchain_colorspace";
const VK_KHR_GET_PHYSICAL_DEVICE_PROPERTIES_2_EXTENSION_NAME: &CStr =
    c"VK_KHR_get_physical_device_properties2";
const VK_KHR_PORTABILITY_ENUMERATION_EXTENSION_NAME: &CStr = c"VK_KHR_portability_enumeration";

/// The device extensions the backend uses. Translation of
/// `VulkanExtensions`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct VulkanExtensions {
    // These extensions are required!

    // Globally supported
    pub(super) khr_swapchain: bool,
    // Core since 1.1, needed for negative VkViewport::height
    pub(super) khr_maintenance1: bool,

    // These extensions are optional!

    // Core since 1.2, but requires annoying paperwork to implement
    pub(super) khr_driver_properties: bool,
    // Only required for special implementations (i.e. MoltenVK)
    pub(super) khr_portability_subset: bool,
    // Only required to detect devices using Dozen D3D12 driver
    pub(super) msft_layered_driver: bool,
    // Only required for decoding HDR ASTC textures
    pub(super) ext_texture_compression_astc_hdr: bool,
}

impl VulkanExtensions {
    /// The extensions, by name, with whether each is supported.
    fn entries(&self) -> [(&'static CStr, bool); 6] {
        [
            (c"VK_KHR_swapchain", self.khr_swapchain),
            (c"VK_KHR_maintenance1", self.khr_maintenance1),
            (c"VK_KHR_driver_properties", self.khr_driver_properties),
            (c"VK_KHR_portability_subset", self.khr_portability_subset),
            (c"VK_MSFT_layered_driver", self.msft_layered_driver),
            (
                c"VK_EXT_texture_compression_astc_hdr",
                self.ext_texture_compression_astc_hdr,
            ),
        ]
    }

    /// The supported extensions of a device's, and whether the required
    /// ones are there. Translation of `CheckDeviceExtensions()`.
    pub(super) fn check(extensions: &[VkExtensionProperties]) -> (VulkanExtensions, bool) {
        let mut supports = VulkanExtensions::default();
        for extension in extensions {
            match extension.name().as_str() {
                "VK_KHR_swapchain" => supports.khr_swapchain = true,
                "VK_KHR_maintenance1" => supports.khr_maintenance1 = true,
                "VK_KHR_driver_properties" => supports.khr_driver_properties = true,
                "VK_KHR_portability_subset" => supports.khr_portability_subset = true,
                "VK_MSFT_layered_driver" => supports.msft_layered_driver = true,
                "VK_EXT_texture_compression_astc_hdr" => {
                    supports.ext_texture_compression_astc_hdr = true
                }
                _ => {}
            }
        }

        let required = supports.khr_swapchain && supports.khr_maintenance1;
        (supports, required)
    }

    /// The names of the supported extensions, in upstream's order.
    /// Translation of `GetDeviceExtensionCount()` and
    /// `CreateDeviceExtensionArray()`.
    pub(super) fn names(&self) -> Vec<&'static CStr> {
        self.entries()
            .into_iter()
            .filter(|&(_, supported)| supported)
            .map(|(name, _)| name)
            .collect()
    }
}

/// Translation of `VulkanFeatures` (zeroed by default, as upstream's).
#[derive(Default)]
pub(super) struct VulkanFeatures {
    pub(super) desired_api_version: u32,
    pub(super) desired_vulkan10_device_features: VkPhysicalDeviceFeatures,
    pub(super) desired_vulkan11_device_features: VkPhysicalDeviceVulkan11Features,
    pub(super) desired_vulkan12_device_features: VkPhysicalDeviceVulkan12Features,
    pub(super) desired_vulkan13_device_features: VkPhysicalDeviceVulkan13Features,

    pub(super) uses_custom_vulkan_options: bool,

    pub(super) additional_device_extension_names: Vec<CString>,
    pub(super) additional_instance_extension_names: Vec<CString>,
}

/// What `VULKAN_INTERNAL_PrepareVulkan()` leaves in the renderer: the
/// loader's functions, the instance and the physical device chosen.
pub(super) struct PreparedVulkan {
    pub(super) vk_get_instance_proc_addr: PfnVkGetInstanceProcAddr,
    pub(super) global: GlobalFunctions,
    pub(super) instance: VkInstance,
    pub(super) inst: InstanceFunctions,

    pub(super) debug_mode: bool,
    pub(super) prefer_low_power: bool,
    pub(super) require_hardware_acceleration: bool,

    pub(super) supports_debug_utils: bool,
    pub(super) supports_colorspace: bool,
    pub(super) supports_physical_device_properties2: bool,
    pub(super) supports_portability_enumeration: bool,

    pub(super) physical_device: VkPhysicalDevice,
    pub(super) supports: VulkanExtensions,
    pub(super) queue_family_index: u32,
    pub(super) physical_device_properties: VkPhysicalDeviceProperties2,
    pub(super) physical_device_driver_properties: VkPhysicalDeviceDriverProperties,
    pub(super) memory_properties: VkPhysicalDeviceMemoryProperties,
}

/// Call a Vulkan enumeration function twice: for the count, then for the
/// items. Upstream ignores the results of these calls.
///
/// # Safety
///
/// `f` is a Vulkan enumeration function's call with its count and array
/// pointers.
unsafe fn enumerate<T: Default + Clone>(mut f: impl FnMut(*mut u32, *mut T) -> VkResult) -> Vec<T> {
    let mut count = 0;
    let _ = f(&mut count, null_mut());
    let mut items = vec![T::default(); count as usize];
    let _ = f(&mut count, items.as_mut_ptr());
    items.truncate(count as usize);
    items
}

/// The text of a fixed-size name of a Vulkan structure.
fn c_name(chars: &[c_char]) -> String {
    c_chars_to_string(chars)
}

/// Translation of `SupportsInstanceExtension()`.
fn supports_instance_extension(ext: &CStr, available_extensions: &[VkExtensionProperties]) -> bool {
    let ext = ext.to_string_lossy();
    available_extensions.iter().any(|e| e.name() == ext)
}

/// The optional instance extensions `VULKAN_INTERNAL_CheckInstanceExtensions()`
/// reports.
#[derive(Clone, Copy, Debug, Default)]
struct InstanceExtensionSupport {
    debug_utils: bool,
    colorspace: bool,
    physical_device_properties2: bool,
    portability_enumeration: bool,
}

/// Translation of `VULKAN_INTERNAL_CheckInstanceExtensions()`: the
/// optional extensions, or the index of the first required one missing.
fn check_instance_extensions(
    global: &GlobalFunctions,
    required_extensions: &[&CStr],
) -> (InstanceExtensionSupport, Option<usize>) {
    // SAFETY: the loader's function with its count and array pointers.
    let available_extensions = unsafe {
        enumerate(|count, items| {
            (global.enumerate_instance_extension_properties)(null(), count, items)
        })
    };

    let first_unsupported = required_extensions
        .iter()
        .position(|ext| !supports_instance_extension(ext, &available_extensions));

    let support = InstanceExtensionSupport {
        // This is optional, but nice to have!
        debug_utils: supports_instance_extension(
            VK_EXT_DEBUG_UTILS_EXTENSION_NAME,
            &available_extensions,
        ),
        // Also optional and nice to have!
        colorspace: supports_instance_extension(
            VK_EXT_SWAPCHAIN_COLOR_SPACE_EXTENSION_NAME,
            &available_extensions,
        ),
        // Only needed for KHR_driver_properties!
        physical_device_properties2: supports_instance_extension(
            VK_KHR_GET_PHYSICAL_DEVICE_PROPERTIES_2_EXTENSION_NAME,
            &available_extensions,
        ),
        // Only needed for MoltenVK!
        portability_enumeration: supports_instance_extension(
            VK_KHR_PORTABILITY_ENUMERATION_EXTENSION_NAME,
            &available_extensions,
        ),
    };

    (support, first_unsupported)
}

/// The first of the opt-in device extensions a device doesn't have.
/// Translation of `CheckOptInDeviceExtensions()`.
fn check_opt_in_device_extensions<'a>(
    features: &'a VulkanFeatures,
    available_extensions: &[VkExtensionProperties],
) -> Option<&'a CString> {
    features
        .additional_device_extension_names
        .iter()
        .find(|name| {
            let name = name.to_string_lossy();
            !available_extensions.iter().any(|e| e.name() == name)
        })
}

/// Translation of `VULKAN_INTERNAL_CheckValidationLayers()`.
fn check_validation_layers(global: &GlobalFunctions, validation_layers: &[&CStr]) -> bool {
    // SAFETY: the loader's function with its count and array pointers.
    let available_layers = unsafe {
        enumerate(|count, items| (global.enumerate_instance_layer_properties)(count, items))
    };

    let mut layer_found = false;
    for layer in validation_layers {
        let layer = layer.to_string_lossy();
        layer_found = available_layers
            .iter()
            .any(|l| c_name(&l.layer_name) == layer);

        if !layer_found {
            break;
        }
    }

    layer_found
}

/// The features `requested` has that `supported` doesn't, logged by name;
/// whether there are none. The `CHECK_OPTIONAL_DEVICE_FEATURE` lists of
/// `VULKAN_INTERNAL_ValidateOptInVulkan1xFeatures()`, whose lists name
/// every feature of the structures.
fn validate_opt_in_features(
    requested: &[VkBool32],
    supported: &[VkBool32],
    names: &[&str],
) -> bool {
    let mut result = true;
    for ((&requested, &supported), name) in requested.iter().zip(supported).zip(names) {
        if requested != 0 && supported == 0 {
            crate::log::verbose!(
                Category::Gpu,
                "SDL GPU Vulkan: Application requested unsupported physical device feature '{}'",
                name
            );
            result = false;
        }
    }
    result
}

/// `*first |= *firstToAdd` over a range of features. Translation of
/// `VULKAN_INTERNAL_AddDeviceFeatures()` (over all the members, where
/// upstream passes the first and last).
///
/// Note (upstream): the members missing from a shorter list are false.
fn add_device_features(dst: Vec<&mut VkBool32>, src: &[bool]) {
    for (d, &s) in dst.into_iter().zip(src) {
        *d |= s as VkBool32;
    }
}

/// Translation of `VULKAN_INTERNAL_TryAddDeviceFeatures_Vulkan_11()`.
fn try_add_device_features_vulkan_11(
    dst10: &mut VkPhysicalDeviceFeatures,
    dst11: &mut VkPhysicalDeviceVulkan11Features,
    src: &VulkanFeatureStructure,
) -> bool {
    let f = |i: usize| src.features.get(i).copied().unwrap_or(false) as VkBool32;
    match src.s_type {
        VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2 => {
            add_device_features(dst10.bools_mut(), &src.features);
        }
        VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_16BIT_STORAGE_FEATURES => {
            dst11.storage_buffer16_bit_access |= f(0);
            dst11.uniform_and_storage_buffer16_bit_access |= f(1);
            dst11.storage_push_constant16 |= f(2);
            dst11.storage_input_output16 |= f(3);
        }
        VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTIVIEW_FEATURES => {
            dst11.multiview |= f(0);
            dst11.multiview_geometry_shader |= f(1);
            dst11.multiview_tessellation_shader |= f(2);
        }
        VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROTECTED_MEMORY_FEATURES => {
            dst11.protected_memory |= f(0);
        }
        VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLER_YCBCR_CONVERSION_FEATURES => {
            dst11.sampler_ycbcr_conversion |= f(0);
        }
        VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_DRAW_PARAMETERS_FEATURES => {
            dst11.shader_draw_parameters |= f(0);
        }
        VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VARIABLE_POINTERS_FEATURES => {
            // (variablePointersStorageBuffer comes first in the structure)
            dst11.variable_pointers |= f(1);
            dst11.variable_pointers_storage_buffer |= f(0);
        }
        _ => return false,
    }
    true
}

/// Translation of `VULKAN_INTERNAL_TryAddDeviceFeatures_Vulkan_12_Or_Later()`.
fn try_add_device_features_vulkan_12_or_later(
    dst10: &mut VkPhysicalDeviceFeatures,
    dst11: &mut VkPhysicalDeviceVulkan11Features,
    dst12: &mut VkPhysicalDeviceVulkan12Features,
    dst13: &mut VkPhysicalDeviceVulkan13Features,
    api_version: u32,
    src: &VulkanFeatureStructure,
) -> bool {
    let minor_version = vk_api_version_minor(api_version);
    crate::sdl_assert!(api_version >= 2);
    let mut has_added = try_add_device_features_vulkan_11(dst10, dst11, src);
    if !has_added {
        match src.s_type {
            VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES => {
                add_device_features(dst11.bools_mut(), &src.features);
                has_added = true;
            }
            VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES => {
                add_device_features(dst12.bools_mut(), &src.features);
                has_added = true;
            }
            VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES if minor_version >= 3 => {
                add_device_features(dst13.bools_mut(), &src.features);
                has_added = true;
            }
            _ => {}
        }
    }

    has_added
}

/// Take the opt-in options of `SDL_PROP_GPU_DEVICE_CREATE_VULKAN_OPTIONS_POINTER`.
/// Translation of `VULKAN_INTERNAL_AddOptInVulkanOptions()`.
pub(super) fn add_opt_in_vulkan_options(
    props: &Properties,
    debug_mode: bool,
    features: &mut VulkanFeatures,
) {
    if !props.contains(PROP_GPU_DEVICE_CREATE_VULKAN_OPTIONS_POINTER) {
        return;
    }
    let Some(options) =
        props.get_any::<VulkanOptions>(PROP_GPU_DEVICE_CREATE_VULKAN_OPTIONS_POINTER)
    else {
        if debug_mode {
            crate::log::warn!(
                Category::Gpu,
                "VULKAN_INTERNAL_AddOptInVulkanOptions: Additional options property was set, but value was null. This may be a bug."
            );
        }
        return;
    };

    features.uses_custom_vulkan_options = true;
    features.desired_api_version = options.vulkan_api_version;

    features.desired_vulkan11_device_features = VkPhysicalDeviceVulkan11Features {
        s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES,
        ..Default::default()
    };
    features.desired_vulkan12_device_features = VkPhysicalDeviceVulkan12Features {
        s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES,
        ..Default::default()
    };
    features.desired_vulkan13_device_features = VkPhysicalDeviceVulkan13Features {
        s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES,
        ..Default::default()
    };

    // Handle requested device features
    if let Some(device_features) = &options.vulkan_10_physical_device_features {
        add_device_features(
            features.desired_vulkan10_device_features.bools_mut(),
            device_features,
        );
    }

    let minor_version = vk_api_version_minor(features.desired_api_version);
    let supports_higher_level_features = minor_version > 0;
    if supports_higher_level_features {
        // Iterate through the entire list and combine all requested features
        for next_structure in &options.feature_list {
            if minor_version < 2 {
                try_add_device_features_vulkan_11(
                    &mut features.desired_vulkan10_device_features,
                    &mut features.desired_vulkan11_device_features,
                    next_structure,
                );
            } else {
                try_add_device_features_vulkan_12_or_later(
                    &mut features.desired_vulkan10_device_features,
                    &mut features.desired_vulkan11_device_features,
                    &mut features.desired_vulkan12_device_features,
                    &mut features.desired_vulkan13_device_features,
                    features.desired_api_version,
                    next_structure,
                );
            }
        }
    }

    // (a name with a NUL byte becomes an empty name, which no device or
    // instance supports)
    let c_strings = |names: &[String]| -> Vec<CString> {
        names
            .iter()
            .map(|n| CString::new(n.as_str()).unwrap_or_default())
            .collect()
    };
    features.additional_device_extension_names = c_strings(&options.device_extension_names);
    features.additional_instance_extension_names = c_strings(&options.instance_extension_names);
}

/// Translation of `VULKAN_INTERNAL_LoadEntryPoints()`: the loader's
/// `vkGetInstanceProcAddr` and the global functions.
///
/// Note (upstream): C logs a failure and goes on with NULL functions
/// (calling them next); the failure is returned here.
fn load_entry_points() -> Result<(PfnVkGetInstanceProcAddr, GlobalFunctions)> {
    // Required for MoltenVK support
    let _ = crate::stdlib::setenv_unsafe("MVK_CONFIG_FULL_IMAGE_VIEW_SWIZZLE", "1", true);

    // Load Vulkan entry points
    // FIXME (upstream): this reference to the loader is never released
    // (each PrepareDriver and CreateDevice takes one more).
    if let Err(e) = crate::video::vulkan::vulkan_load_library(None) {
        crate::log::warn!(Category::Gpu, "Vulkan: SDL_Vulkan_LoadLibrary failed!");
        return Err(e);
    }

    let address = match crate::video::vulkan::vulkan_get_vk_get_instance_proc_addr() {
        Ok(address) if address != 0 => address,
        other => {
            let e = other
                .err()
                .unwrap_or_else(|| Error::new("No Vulkan loader has been loaded"));
            crate::log::warn!(
                Category::Gpu,
                "SDL_Vulkan_GetVkGetInstanceProcAddr(): {}",
                e.message()
            );
            return Err(e);
        }
    };
    // SAFETY: the video driver's vkGetInstanceProcAddr, which stays loaded
    // while the reference to the loader is held.
    let vk_get_instance_proc_addr =
        unsafe { std::mem::transmute::<usize, PfnVkGetInstanceProcAddr>(address) };

    let lookup = |name: &CStr| {
        // SAFETY: the loader's vkGetInstanceProcAddr with a null instance
        // and a NUL-terminated name.
        unsafe { vk_get_instance_proc_addr(null_mut(), name.as_ptr()) }
    };
    // SAFETY: vkGetInstanceProcAddr returns the functions of the names.
    match unsafe { GlobalFunctions::load(lookup) } {
        Ok(global) => Ok((vk_get_instance_proc_addr, global)),
        Err(name) => {
            let message = format!("vkGetInstanceProcAddr(VK_NULL_HANDLE, \"{name}\") failed");
            crate::log::warn!(Category::Gpu, "{}", message);
            Err(Error::new(message))
        }
    }
}

/// Translation of `VULKAN_INTERNAL_CreateInstance()`: the instance, and
/// the optional instance extensions it has.
fn create_instance(
    debug_mode: bool,
    global: &GlobalFunctions,
    features: &VulkanFeatures,
) -> Result<(VkInstance, InstanceExtensionSupport)> {
    let layer_names = [VALIDATION_LAYER_NAME.as_ptr()];

    let app_info = VkApplicationInfo {
        s_type: VK_STRUCTURE_TYPE_APPLICATION_INFO,
        p_next: null(),
        p_application_name: null(),
        application_version: 0,
        p_engine_name: c"SDLGPU".as_ptr(),
        engine_version: crate::version::Version::CURRENT.to_number() as u32,
        api_version: if features.uses_custom_vulkan_options {
            features.desired_api_version
        } else {
            vk_make_version(1, 0, 0)
        },
    };

    let mut create_flags = 0;

    let original_instance_extension_names = match crate::video::vulkan::vulkan_instance_extensions()
    {
        Ok(names) => names,
        Err(e) => {
            crate::log::error!(
                Category::Gpu,
                "SDL_Vulkan_GetInstanceExtensions(): getExtensionCount: {}",
                e.message()
            );
            return Err(e);
        }
    };

    /* Extra space for the following extensions:
     * VK_KHR_get_physical_device_properties2
     * VK_EXT_swapchain_colorspace
     * VK_EXT_debug_utils
     * VK_KHR_portability_enumeration
     *
     * Plus additional opt-in extensions.
     */
    let mut instance_extension_names: Vec<CString> = original_instance_extension_names
        .iter()
        .map(|n| CString::new(*n).unwrap_or_default())
        .collect();
    instance_extension_names.extend(features.additional_instance_extension_names.iter().cloned());

    let required: Vec<&CStr> = instance_extension_names
        .iter()
        .map(|n| n.as_c_str())
        .collect();
    let (support, first_unsupported) = check_instance_extensions(global, &required);
    if let Some(first_unsupported_extension_index) = first_unsupported {
        let message = format!(
            "Required Vulkan instance extension '{}' not supported",
            instance_extension_names[first_unsupported_extension_index].to_string_lossy()
        );
        if debug_mode {
            crate::log::error!(Category::Gpu, "{}", message);
        }
        return Err(Error::new(message));
    }

    if support.debug_utils {
        // Append the debug extension
        instance_extension_names.push(VK_EXT_DEBUG_UTILS_EXTENSION_NAME.into());
    } else {
        crate::log::warn!(
            Category::Gpu,
            "{} is not supported!",
            VK_EXT_DEBUG_UTILS_EXTENSION_NAME.to_string_lossy()
        );
    }

    if support.colorspace {
        // Append colorspace extension
        instance_extension_names.push(VK_EXT_SWAPCHAIN_COLOR_SPACE_EXTENSION_NAME.into());
    }

    if support.physical_device_properties2 {
        // Append KHR_physical_device_properties2 extension
        instance_extension_names
            .push(VK_KHR_GET_PHYSICAL_DEVICE_PROPERTIES_2_EXTENSION_NAME.into());
    }

    if support.portability_enumeration {
        instance_extension_names.push(VK_KHR_PORTABILITY_ENUMERATION_EXTENSION_NAME.into());
        create_flags |= VK_INSTANCE_CREATE_ENUMERATE_PORTABILITY_BIT_KHR;
    }

    let extension_pointers: Vec<*const c_char> = instance_extension_names
        .iter()
        .map(|n| n.as_ptr())
        .collect();

    let mut enabled_layer_count = 0;
    if debug_mode {
        enabled_layer_count = layer_names.len() as u32;
        if !check_validation_layers(global, &[VALIDATION_LAYER_NAME]) {
            crate::log::warn!(
                Category::Gpu,
                "Validation layers not found, continuing without validation"
            );
            enabled_layer_count = 0;
        } else {
            crate::log::info!(
                Category::Gpu,
                "Validation layers enabled, expect debug level performance!"
            );
        }
    }

    let create_info = VkInstanceCreateInfo {
        s_type: VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
        p_next: null(),
        flags: create_flags,
        p_application_info: &app_info,
        enabled_layer_count,
        pp_enabled_layer_names: layer_names.as_ptr(),
        enabled_extension_count: extension_pointers.len() as u32,
        pp_enabled_extension_names: extension_pointers.as_ptr(),
    };

    // (HAVE_GPU_OPENXR: OpenXR is not translated.)
    let mut instance = null_mut();
    // SAFETY: a valid create info whose strings outlive the call.
    let vulkan_result = unsafe { (global.create_instance)(&create_info, null(), &mut instance) };

    if vulkan_result != VK_SUCCESS {
        return Err(vulkan_error(debug_mode, vulkan_result, "vkCreateInstance"));
    }

    Ok((instance, support))
}

/// The VkPhysicalDeviceType priorities (`DEVICE_PRIORITY_HIGHPERFORMANCE`).
const DEVICE_PRIORITY_HIGHPERFORMANCE: [u8; 5] = [
    0, // VK_PHYSICAL_DEVICE_TYPE_OTHER
    3, // VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU
    4, // VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU
    2, // VK_PHYSICAL_DEVICE_TYPE_VIRTUAL_GPU
    1, // VK_PHYSICAL_DEVICE_TYPE_CPU
];
/// `DEVICE_PRIORITY_LOWPOWER`
const DEVICE_PRIORITY_LOWPOWER: [u8; 5] = [
    0, // VK_PHYSICAL_DEVICE_TYPE_OTHER
    4, // VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU
    3, // VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU
    2, // VK_PHYSICAL_DEVICE_TYPE_VIRTUAL_GPU
    1, // VK_PHYSICAL_DEVICE_TYPE_CPU
];

impl PreparedVulkan {
    /// `vkGetPhysicalDeviceProperties2KHR`, or (Note (upstream): C calls it
    /// without checking that the instance has it) the plain properties.
    fn get_physical_device_properties2(
        &self,
        physical_device: VkPhysicalDevice,
        properties: &mut VkPhysicalDeviceProperties2,
    ) {
        match self.inst.get_physical_device_properties2_khr {
            // SAFETY: a physical device of the instance and a valid chain.
            Some(f) => unsafe { f(physical_device, properties) },
            // SAFETY: as above.
            None => unsafe {
                (self.inst.get_physical_device_properties)(
                    physical_device,
                    &mut properties.properties,
                )
            },
        }
    }

    /// The rank of a device (0 and false for a device outranked or not
    /// suitable). Translation of `VULKAN_INTERNAL_GetDeviceRank()`.
    fn get_device_rank(
        &self,
        physical_device: VkPhysicalDevice,
        physical_device_extensions: &VulkanExtensions,
        device_rank: &mut u64,
    ) -> bool {
        let device_priority = if self.prefer_low_power {
            &DEVICE_PRIORITY_LOWPOWER
        } else {
            &DEVICE_PRIORITY_HIGHPERFORMANCE
        };
        let mut is_conformant;

        let device_type;
        if physical_device_extensions.khr_driver_properties
            || physical_device_extensions.msft_layered_driver
        {
            let mut physical_device_properties = VkPhysicalDeviceProperties2 {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2,
                ..Default::default()
            };
            let mut physical_device_driver_properties = VkPhysicalDeviceDriverProperties::default();
            let mut physical_device_layered_driver_properties =
                VkPhysicalDeviceLayeredDriverPropertiesMSFT::default();
            let mut pp_next: *mut *mut c_void = &mut physical_device_properties.p_next;

            if physical_device_extensions.khr_driver_properties {
                physical_device_driver_properties.s_type =
                    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRIVER_PROPERTIES;
                // SAFETY: pp_next points at a live p_next field.
                unsafe {
                    *pp_next = std::ptr::from_mut(&mut physical_device_driver_properties).cast()
                };
                pp_next = &mut physical_device_driver_properties.p_next;
            }

            if physical_device_extensions.msft_layered_driver {
                physical_device_layered_driver_properties.s_type =
                    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LAYERED_DRIVER_PROPERTIES_MSFT;
                // SAFETY: as above.
                unsafe {
                    *pp_next =
                        std::ptr::from_mut(&mut physical_device_layered_driver_properties).cast()
                };
                pp_next = &mut physical_device_layered_driver_properties.p_next;
            }

            // SAFETY: as above.
            unsafe { *pp_next = null_mut() };
            self.get_physical_device_properties2(physical_device, &mut physical_device_properties);

            if physical_device_extensions.khr_driver_properties {
                is_conformant = physical_device_driver_properties.conformance_version.major >= 1;
            } else {
                is_conformant = true; // We can't check this, so just assume it's conformant
            }

            if physical_device_extensions.msft_layered_driver
                && physical_device_layered_driver_properties.underlying_api
                    != VK_LAYERED_DRIVER_UNDERLYING_API_NONE_MSFT
            {
                /* Rank Dozen above CPU, but below INTEGRATED.
                 * This is needed for WSL specifically.
                 */
                device_type = VK_PHYSICAL_DEVICE_TYPE_VIRTUAL_GPU;

                /* Dozen hasn't been tested for conformance and it probably won't be,
                 * but WSL may need this so let's be generous.
                 * -flibit
                 */
                is_conformant = true;
            } else {
                device_type = physical_device_properties.properties.device_type;
            }
        } else {
            let mut physical_device_properties = VkPhysicalDeviceProperties::default();
            // SAFETY: a physical device of the instance.
            unsafe {
                (self.inst.get_physical_device_properties)(
                    physical_device,
                    &mut physical_device_properties,
                )
            };
            device_type = physical_device_properties.device_type;
            is_conformant = true; // We can't check this, so just assume it's conformant
        }

        if self.require_hardware_acceleration
            && device_type != VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU
            && device_type != VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU
            && device_type != VK_PHYSICAL_DEVICE_TYPE_VIRTUAL_GPU
        {
            // In addition to CPU, "Other" drivers (including layered drivers) don't count as hardware-accelerated
            return false;
        }

        /* As far as I know, the only drivers available to users that are also
         * non-conformant are incomplete Mesa drivers and Vulkan-on-12. hasvk is one
         * example of a non-conformant driver that's built by default.
         * -flibit
         */
        if !is_conformant {
            return false;
        }

        /* Apply a large bias on the devicePriority so that we always respect the order in the priority arrays.
         * We also rank by e.g. VRAM which should have less influence than the device type.
         */
        // Note (upstream): C indexes the table with the device type
        // unchecked; a type past it ranks as OTHER here.
        let device_priority_value = device_priority
            .get(device_type as usize)
            .copied()
            .unwrap_or(0) as u64
            * 1000000;

        if *device_rank < device_priority_value {
            /* This device outranks the best device we've found so far!
             * This includes a dedicated GPU that has less features than an
             * integrated GPU, because this is a freak case that is almost
             * never intentionally desired by the end user
             */
            *device_rank = device_priority_value;
        } else if *device_rank > device_priority_value {
            /* Device is outranked by a previous device, don't even try to
             * run a query and reset the rank to avoid overwrites
             */
            *device_rank = 0;
            return false;
        }

        /* If we prefer high performance, sum up all device local memory (rounded to megabytes)
         * to deviceRank. In the niche case of someone having multiple dedicated GPUs in the same
         * system, this theoretically picks the most powerful one (or at least the one with the
         * most memory!)
         *
         * We do this *after* discarding all non suitable devices, which means if this computer
         * has multiple dedicated GPUs that all meet our criteria, *and* the user asked for high
         * performance, then we always pick the GPU with more VRAM.
         */
        if !self.prefer_low_power {
            let mut device_memory = VkPhysicalDeviceMemoryProperties::default();
            // SAFETY: a physical device of the instance.
            unsafe {
                (self.inst.get_physical_device_memory_properties)(
                    physical_device,
                    &mut device_memory,
                )
            };
            let video_memory: u64 = device_memory.memory_heaps
                [..(device_memory.memory_heap_count as usize).min(VK_MAX_MEMORY_HEAPS)]
                .iter()
                .filter(|heap| heap.flags & VK_MEMORY_HEAP_DEVICE_LOCAL_BIT != 0)
                .map(|heap| heap.size)
                .sum();
            // Round it to megabytes (as per the vulkan spec videoMemory is in bytes)
            let video_memory_rounded = video_memory / 1024 / 1024;
            *device_rank += video_memory_rounded;
        }

        true
    }

    /// The opt-in features a device doesn't support. Translation of
    /// `VULKAN_INTERNAL_ValidateOptInFeatures()`.
    fn validate_opt_in_features(
        &self,
        features: &VulkanFeatures,
        physical_device: VkPhysicalDevice,
        vk10_features: &VkPhysicalDeviceFeatures,
    ) -> bool {
        let mut supports_all_features = true;

        let minor_version = vk_api_version_minor(features.desired_api_version);
        let get_features2 = self.inst.get_physical_device_features2;

        let check10 = |supports: &mut bool| {
            *supports &= validate_opt_in_features(
                &features.desired_vulkan10_device_features.bools(),
                &vk10_features.bools(),
                VkPhysicalDeviceFeatures::NAMES,
            );
        };

        if minor_version < 1 {
            check10(&mut supports_all_features);
        } else if minor_version < 2 {
            // Query device features using the pre-1.2 structures
            let mut storage = VkPhysicalDevice16BitStorageFeatures {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_16BIT_STORAGE_FEATURES,
                ..Default::default()
            };
            let mut multiview = VkPhysicalDeviceMultiviewFeatures {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTIVIEW_FEATURES,
                ..Default::default()
            };
            let mut protected_mem = VkPhysicalDeviceProtectedMemoryFeatures {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROTECTED_MEMORY_FEATURES,
                ..Default::default()
            };
            let mut ycbcr = VkPhysicalDeviceSamplerYcbcrConversionFeatures {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLER_YCBCR_CONVERSION_FEATURES,
                ..Default::default()
            };
            let mut draw_params = VkPhysicalDeviceShaderDrawParametersFeatures {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_DRAW_PARAMETERS_FEATURES,
                ..Default::default()
            };
            let mut var_pointers = VkPhysicalDeviceVariablePointersFeatures {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VARIABLE_POINTERS_FEATURES,
                ..Default::default()
            };
            draw_params.p_next = std::ptr::from_mut(&mut var_pointers).cast();
            ycbcr.p_next = std::ptr::from_mut(&mut draw_params).cast();
            protected_mem.p_next = std::ptr::from_mut(&mut ycbcr).cast();
            multiview.p_next = std::ptr::from_mut(&mut protected_mem).cast();
            storage.p_next = std::ptr::from_mut(&mut multiview).cast();
            let mut supported_feature_list = VkPhysicalDeviceFeatures2 {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2,
                p_next: std::ptr::from_mut(&mut storage).cast(),
                ..Default::default()
            };

            // Note (upstream): C calls vkGetPhysicalDeviceFeatures2 without
            // checking it; without it the features read as unsupported.
            if let Some(f) = get_features2 {
                // SAFETY: a physical device of the instance and a valid
                // chain of live structures.
                unsafe { f(physical_device, &mut supported_feature_list) };
            }

            // Pack the results into the post-1.2 structure for easier checking
            let vk11_features = VkPhysicalDeviceVulkan11Features {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES,
                storage_buffer16_bit_access: storage.storage_buffer16_bit_access,
                uniform_and_storage_buffer16_bit_access: storage
                    .uniform_and_storage_buffer16_bit_access,
                storage_push_constant16: storage.storage_push_constant16,
                storage_input_output16: storage.storage_input_output16,
                multiview: multiview.multiview,
                multiview_geometry_shader: multiview.multiview_geometry_shader,
                multiview_tessellation_shader: multiview.multiview_tessellation_shader,
                protected_memory: protected_mem.protected_memory,
                sampler_ycbcr_conversion: ycbcr.sampler_ycbcr_conversion,
                shader_draw_parameters: draw_params.shader_draw_parameters,
                variable_pointers: var_pointers.variable_pointers,
                variable_pointers_storage_buffer: var_pointers.variable_pointers_storage_buffer,
                ..Default::default()
            };

            // Check support
            check10(&mut supports_all_features);
            supports_all_features &= validate_opt_in_features(
                &features.desired_vulkan11_device_features.bools(),
                &vk11_features.bools(),
                VkPhysicalDeviceVulkan11Features::NAMES,
            );
        } else {
            let mut vk13_features = VkPhysicalDeviceVulkan13Features {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES,
                ..Default::default()
            };
            let mut vk12_features = VkPhysicalDeviceVulkan12Features {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES,
                p_next: std::ptr::from_mut(&mut vk13_features).cast(),
                ..Default::default()
            };
            let mut vk11_features = VkPhysicalDeviceVulkan11Features {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES,
                p_next: std::ptr::from_mut(&mut vk12_features).cast(),
                ..Default::default()
            };
            let mut supported_feature_list = VkPhysicalDeviceFeatures2 {
                s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2,
                p_next: std::ptr::from_mut(&mut vk11_features).cast(),
                ..Default::default()
            };

            if let Some(f) = get_features2 {
                // SAFETY: as above.
                unsafe { f(physical_device, &mut supported_feature_list) };
            }

            check10(&mut supports_all_features);
            supports_all_features &= validate_opt_in_features(
                &features.desired_vulkan11_device_features.bools(),
                &vk11_features.bools(),
                VkPhysicalDeviceVulkan11Features::NAMES,
            );
            supports_all_features &= validate_opt_in_features(
                &features.desired_vulkan12_device_features.bools(),
                &vk12_features.bools(),
                VkPhysicalDeviceVulkan12Features::NAMES,
            );
            supports_all_features &= validate_opt_in_features(
                &features.desired_vulkan13_device_features.bools(),
                &vk13_features.bools(),
                VkPhysicalDeviceVulkan13Features::NAMES,
            );
        }

        supports_all_features
    }

    /// The device's extensions, and whether it has the required ones and
    /// the opt-in ones. Translation of
    /// `VULKAN_INTERNAL_CheckDeviceExtensions()`.
    fn check_device_extensions(
        &self,
        features: &VulkanFeatures,
        physical_device: VkPhysicalDevice,
    ) -> (VulkanExtensions, bool) {
        // SAFETY: a physical device of the instance, with its count and
        // array pointers.
        let available_extensions = unsafe {
            enumerate(|count, items| {
                (self.inst.enumerate_device_extension_properties)(
                    physical_device,
                    null(),
                    count,
                    items,
                )
            })
        };

        let (physical_device_extensions, mut all_extensions_supported) =
            VulkanExtensions::check(&available_extensions);

        if features.uses_custom_vulkan_options {
            if let Some(missing_extension_name) =
                check_opt_in_device_extensions(features, &available_extensions)
            {
                if self.debug_mode {
                    crate::log::error!(
                        Category::Gpu,
                        "Required Vulkan device extension '{}' not supported",
                        missing_extension_name.to_string_lossy()
                    );
                }
                all_extensions_supported = false;
            }
        }

        (physical_device_extensions, all_extensions_supported)
    }

    /// Whether a device has what the backend needs, with its extensions and
    /// the queue family to use. Translation of
    /// `VULKAN_INTERNAL_IsDeviceSuitable()`.
    fn is_device_suitable(
        &self,
        features: &VulkanFeatures,
        physical_device: VkPhysicalDevice,
    ) -> Option<(VulkanExtensions, u32)> {
        let mut device_features = VkPhysicalDeviceFeatures::default();
        // SAFETY: a physical device of the instance.
        unsafe { (self.inst.get_physical_device_features)(physical_device, &mut device_features) };

        let desired = &features.desired_vulkan10_device_features;
        if (device_features.independent_blend == 0 && desired.independent_blend != 0)
            || (device_features.image_cube_array == 0 && desired.image_cube_array != 0)
            || (device_features.depth_clamp == 0 && desired.depth_clamp != 0)
            || (device_features.shader_clip_distance == 0 && desired.shader_clip_distance != 0)
            || (device_features.draw_indirect_first_instance == 0
                && desired.draw_indirect_first_instance != 0)
            || (device_features.sample_rate_shading == 0 && desired.sample_rate_shading != 0)
            || (device_features.sampler_anisotropy == 0 && desired.sampler_anisotropy != 0)
        {
            return None;
        }

        // Check opt-in device features
        if features.uses_custom_vulkan_options
            && !self.validate_opt_in_features(features, physical_device, &device_features)
        {
            return None;
        }

        let (physical_device_extensions, supported) =
            self.check_device_extensions(features, physical_device);
        if !supported {
            return None;
        }

        // SAFETY: a physical device of the instance, with its count and
        // array pointers.
        let queue_props = unsafe {
            enumerate(|count, items| {
                (self.inst.get_physical_device_queue_family_properties)(
                    physical_device,
                    count,
                    items,
                );
                VK_SUCCESS
            })
        };

        let mut queue_family_best = 0;
        let mut queue_family_index = u32::MAX;
        for (i, props) in (0u32..).zip(&queue_props) {
            let supports_present = crate::video::vulkan::vulkan_presentation_support(
                self.instance as usize,
                physical_device as usize,
                i,
            )
            .unwrap_or(false);
            if !supports_present || (props.queue_flags & VK_QUEUE_GRAPHICS_BIT) == 0 {
                // Not a graphics family, ignore.
                continue;
            }

            /* The queue family bitflags are kind of annoying.
             *
             * We of course need a graphics family, but we ideally want the
             * _primary_ graphics family. The spec states that at least one
             * graphics family must also be a compute family, so generally
             * drivers make that the first one. But hey, maybe something
             * genuinely can't do compute or something, and FNA doesn't
             * need it, so we'll be open to a non-compute queue family.
             *
             * Additionally, it's common to see the primary queue family
             * have the transfer bit set, which is great! But this is
             * actually optional; it's impossible to NOT have transfers in
             * graphics/compute but it _is_ possible for a graphics/compute
             * family, even the primary one, to just decide not to set the
             * bitflag. Admittedly, a driver may want to isolate transfer
             * queues to a dedicated family so that queues made solely for
             * transfers can have an optimized DMA queue.
             *
             * That, or the driver author got lazy and decided not to set
             * the bit. Looking at you, Android.
             *
             * -flibit
             */
            let queue_family_rank = if props.queue_flags & VK_QUEUE_COMPUTE_BIT != 0 {
                if props.queue_flags & VK_QUEUE_TRANSFER_BIT != 0 {
                    // Has all attribs!
                    3
                } else {
                    // Probably has a DMA transfer queue family
                    2
                }
            } else {
                // Just a graphics family, probably has something better
                1
            };
            if queue_family_rank > queue_family_best {
                queue_family_index = i;
                queue_family_best = queue_family_rank;
            }
        }

        if queue_family_index == u32::MAX {
            // Somehow no graphics queues existed. Compute-only device?
            return None;
        }

        // FIXME: Need better structure for checking vs storing swapchain support details
        Some((physical_device_extensions, queue_family_index))
    }

    /// Choose the physical device. Translation of
    /// `VULKAN_INTERNAL_DeterminePhysicalDevice()`.
    fn determine_physical_device(&mut self, features: &VulkanFeatures) -> bool {
        // (HAVE_GPU_OPENXR: OpenXR is not translated.)
        let mut physical_device_count = 0;
        // SAFETY: the instance, with a null array to query the count.
        let vulkan_result = unsafe {
            (self.inst.enumerate_physical_devices)(
                self.instance,
                &mut physical_device_count,
                null_mut(),
            )
        };
        if vulkan_result != VK_SUCCESS {
            let _ = vulkan_error(self.debug_mode, vulkan_result, "vkEnumeratePhysicalDevices");
            return false;
        }

        if physical_device_count == 0 {
            crate::log::info!(Category::Gpu, "Failed to find any GPUs with Vulkan support");
            return false;
        }

        let mut physical_devices = vec![null_mut(); physical_device_count as usize];

        // SAFETY: the instance, with room for the devices.
        let mut vulkan_result = unsafe {
            (self.inst.enumerate_physical_devices)(
                self.instance,
                &mut physical_device_count,
                physical_devices.as_mut_ptr(),
            )
        };
        physical_devices.truncate(physical_device_count as usize);

        /* This should be impossible to hit, but from what I can tell this can
         * be triggered not because the array is too small, but because there
         * were drivers that turned out to be bogus, so this is the loader's way
         * of telling us that the list is now smaller than expected :shrug:
         */
        if vulkan_result == VK_INCOMPLETE {
            crate::log::warn!(
                Category::Gpu,
                "vkEnumeratePhysicalDevices returned VK_INCOMPLETE, will keep trying anyway..."
            );
            vulkan_result = VK_SUCCESS;
        }

        if vulkan_result != VK_SUCCESS {
            crate::log::warn!(
                Category::Gpu,
                "vkEnumeratePhysicalDevices failed: {}",
                vk_error_messages(vulkan_result)
            );
            return false;
        }

        // Any suitable device will do, but we'd like the best
        let mut suitable: Option<(usize, VulkanExtensions, u32)> = None;
        let mut highest_rank = 0;
        for (i, &physical_device) in physical_devices.iter().enumerate() {
            let Some((physical_device_extensions, queue_family_index)) =
                self.is_device_suitable(features, physical_device)
            else {
                // Device does not meet the minimum requirements, skip it entirely
                continue;
            };

            let mut device_rank = highest_rank;
            if self.get_device_rank(
                physical_device,
                &physical_device_extensions,
                &mut device_rank,
            ) {
                /* Use this for rendering.
                 * Note that this may override a previous device that
                 * supports rendering, but shares the same device rank.
                 */
                suitable = Some((i, physical_device_extensions, queue_family_index));
                highest_rank = device_rank;
            }
        }

        let Some((suitable_index, supports, suitable_queue_family_index)) = suitable else {
            return false;
        };
        self.supports = supports;
        self.physical_device = physical_devices[suitable_index];
        self.queue_family_index = suitable_queue_family_index;

        self.physical_device_properties.s_type = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2;
        if self.supports.khr_driver_properties {
            self.physical_device_driver_properties.s_type =
                VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRIVER_PROPERTIES;
            self.physical_device_driver_properties.p_next = null_mut();

            let mut properties = self.physical_device_properties;
            let mut driver_properties = self.physical_device_driver_properties;
            properties.p_next = std::ptr::from_mut(&mut driver_properties).cast();

            self.get_physical_device_properties2(self.physical_device, &mut properties);

            // (the chain's pointer isn't kept: the structures move)
            properties.p_next = null_mut();
            self.physical_device_properties = properties;
            self.physical_device_driver_properties = driver_properties;
        } else {
            self.physical_device_properties.p_next = null_mut();

            // SAFETY: the chosen physical device of the instance.
            unsafe {
                (self.inst.get_physical_device_properties)(
                    self.physical_device,
                    &mut self.physical_device_properties.properties,
                )
            };
        }

        // SAFETY: as above.
        unsafe {
            (self.inst.get_physical_device_memory_properties)(
                self.physical_device,
                &mut self.memory_properties,
            )
        };

        true
    }

    /// Create the logical device and look up its functions; the device,
    /// its functions, its queue and whether fillModeNonSolid and
    /// multiDrawIndirect are on. Translation of
    /// `VULKAN_INTERNAL_CreateLogicalDevice()`.
    fn create_logical_device(
        &self,
        features: &mut VulkanFeatures,
    ) -> Result<(VkDevice, DeviceFunctions, VkQueue, bool, bool)> {
        let queue_priority: f32 = 1.0;

        let queue_create_info = VkDeviceQueueCreateInfo {
            s_type: VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
            p_next: null(),
            flags: 0,
            queue_family_index: self.queue_family_index,
            queue_count: 1,
            p_queue_priorities: &queue_priority,
        };

        // check feature support

        let mut have_device_features = VkPhysicalDeviceFeatures::default();
        // SAFETY: the chosen physical device.
        unsafe {
            (self.inst.get_physical_device_features)(
                self.physical_device,
                &mut have_device_features,
            )
        };

        // specifying used device features

        let mut supports_fill_mode_non_solid = false;
        let mut supports_multi_draw_indirect = false;

        if have_device_features.fill_mode_non_solid != 0 {
            features
                .desired_vulkan10_device_features
                .fill_mode_non_solid = VK_TRUE;
            supports_fill_mode_non_solid = true;
        }

        if have_device_features.multi_draw_indirect != 0 {
            features
                .desired_vulkan10_device_features
                .multi_draw_indirect = VK_TRUE;
            supports_multi_draw_indirect = true;
        }

        // creating the logical device

        let mut portability_features = VkPhysicalDevicePortabilitySubsetFeaturesKHR {
            s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PORTABILITY_SUBSET_FEATURES_KHR,
            p_next: null_mut(),
            constant_alpha_color_blend_factors: VK_FALSE,
            events: VK_FALSE,
            image_view_format_reinterpretation: VK_FALSE,
            image_view_format_swizzle: VK_TRUE,
            image_view2d_on3d_image: VK_FALSE,
            multisample_array_image: VK_FALSE,
            mutable_comparison_samplers: VK_FALSE,
            point_polygons: VK_FALSE,
            sampler_mip_lod_bias: VK_FALSE, // Technically should be true, but eh
            separate_stencil_mask_ref: VK_FALSE,
            shader_sample_rate_interpolation_functions: VK_FALSE,
            tessellation_isolines: VK_FALSE,
            tessellation_point_mode: VK_FALSE,
            triangle_fans: VK_FALSE,
            vertex_attribute_access_beyond_stride: VK_FALSE,
        };

        let mut device_create_info = VkDeviceCreateInfo {
            s_type: VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
            p_next: if self.supports.khr_portability_subset {
                (&mut portability_features as *mut VkPhysicalDevicePortabilitySubsetFeaturesKHR)
                    .cast()
            } else {
                null()
            },
            flags: 0,
            queue_create_info_count: 1,
            p_queue_create_infos: &queue_create_info,
            enabled_layer_count: 0,
            pp_enabled_layer_names: null(),
            ..Default::default()
        };

        // Create the list of device extensions to enable (internal extension + opt-in extensions)
        let mut device_extensions: Vec<*const c_char> =
            self.supports.names().iter().map(|n| n.as_ptr()).collect();
        device_extensions.extend(
            features
                .additional_device_extension_names
                .iter()
                .map(|n| n.as_ptr()),
        );

        device_create_info.enabled_extension_count = device_extensions.len() as u32;
        device_create_info.pp_enabled_extension_names = device_extensions.as_ptr();

        let minor = vk_api_version_minor(features.desired_api_version);

        let mut feature_list = VkPhysicalDeviceFeatures2::default();
        let mut storage = VkPhysicalDevice16BitStorageFeatures::default();
        let mut multiview = VkPhysicalDeviceMultiviewFeatures::default();
        let mut protected_mem = VkPhysicalDeviceProtectedMemoryFeatures::default();
        let mut ycbcr = VkPhysicalDeviceSamplerYcbcrConversionFeatures::default();
        let mut draw_params = VkPhysicalDeviceShaderDrawParametersFeatures::default();
        let mut var_pointers = VkPhysicalDeviceVariablePointersFeatures::default();

        if features.uses_custom_vulkan_options && minor > 0 {
            feature_list.s_type = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2;
            feature_list.features = features.desired_vulkan10_device_features;
            if minor > 1 {
                feature_list.p_next =
                    std::ptr::from_mut(&mut features.desired_vulkan11_device_features).cast();
                features.desired_vulkan11_device_features.p_next =
                    std::ptr::from_mut(&mut features.desired_vulkan12_device_features).cast();
                features.desired_vulkan12_device_features.p_next = if minor > 2 {
                    std::ptr::from_mut(&mut features.desired_vulkan13_device_features).cast()
                } else {
                    null_mut()
                };
                features.desired_vulkan13_device_features.p_next = null_mut();
            } else {
                // Break VkPhysicalDeviceVulkan11Features into pre 1.2 structures for Vulkan 1.1 Support
                let v11 = &features.desired_vulkan11_device_features;

                storage.s_type = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_16BIT_STORAGE_FEATURES;
                storage.storage_buffer16_bit_access = v11.storage_buffer16_bit_access;
                storage.storage_input_output16 = v11.storage_input_output16;
                storage.storage_push_constant16 = v11.storage_push_constant16;
                storage.uniform_and_storage_buffer16_bit_access =
                    v11.uniform_and_storage_buffer16_bit_access;

                multiview.s_type = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTIVIEW_FEATURES;
                multiview.multiview = v11.multiview;
                multiview.multiview_geometry_shader = v11.multiview_geometry_shader;
                multiview.multiview_tessellation_shader = v11.multiview_tessellation_shader;

                protected_mem.s_type = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROTECTED_MEMORY_FEATURES;
                protected_mem.protected_memory = v11.protected_memory;

                ycbcr.s_type = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLER_YCBCR_CONVERSION_FEATURES;
                ycbcr.sampler_ycbcr_conversion = v11.sampler_ycbcr_conversion;

                draw_params.s_type =
                    VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_DRAW_PARAMETERS_FEATURES;
                draw_params.shader_draw_parameters = v11.shader_draw_parameters;

                var_pointers.s_type = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VARIABLE_POINTERS_FEATURES;
                var_pointers.variable_pointers = v11.variable_pointers;
                var_pointers.variable_pointers_storage_buffer =
                    v11.variable_pointers_storage_buffer;

                draw_params.p_next = std::ptr::from_mut(&mut var_pointers).cast();
                ycbcr.p_next = std::ptr::from_mut(&mut draw_params).cast();
                protected_mem.p_next = std::ptr::from_mut(&mut ycbcr).cast();
                multiview.p_next = std::ptr::from_mut(&mut protected_mem).cast();
                storage.p_next = std::ptr::from_mut(&mut multiview).cast();
                feature_list.p_next = std::ptr::from_mut(&mut storage).cast();
            }
            device_create_info.p_enabled_features = null();
            // Note (upstream): this replaces the portability features in
            // the chain, as in C.
            device_create_info.p_next = (&feature_list as *const VkPhysicalDeviceFeatures2).cast();
        } else {
            device_create_info.p_enabled_features = &features.desired_vulkan10_device_features;
        }

        // (HAVE_GPU_OPENXR: OpenXR is not translated.)
        let mut logical_device = null_mut();
        // SAFETY: a valid create info whose chain and arrays outlive the
        // call.
        let vulkan_result = unsafe {
            (self.inst.create_device)(
                self.physical_device,
                &device_create_info,
                null(),
                &mut logical_device,
            )
        };
        if vulkan_result != VK_SUCCESS {
            return Err(vulkan_error(
                self.debug_mode,
                vulkan_result,
                "vkCreateDevice",
            ));
        }

        // Load vkDevice entry points

        let get_device_proc_addr = self.inst.get_device_proc_addr;
        let lookup = |name: &CStr| {
            // SAFETY: the instance's vkGetDeviceProcAddr, with the live
            // device and a NUL-terminated name.
            unsafe { get_device_proc_addr(logical_device, name.as_ptr()) }
        };
        // Note (upstream): C doesn't check the device functions; a missing
        // one fails the device here.
        // SAFETY: vkGetDeviceProcAddr returns the functions of the names.
        let dev = match unsafe { DeviceFunctions::load(lookup) } {
            Ok(dev) => dev,
            Err(name) => {
                return Err(set_string_error(
                    self.debug_mode,
                    &format!("vkGetDeviceProcAddr(device, \"{name}\") failed"),
                ))
            }
        };

        let mut unified_queue = null_mut();
        // SAFETY: the device's queue family, which has a queue.
        unsafe {
            (dev.get_device_queue)(
                logical_device,
                self.queue_family_index,
                0,
                &mut unified_queue,
            )
        };

        Ok((
            logical_device,
            dev,
            unified_queue,
            supports_fill_mode_non_solid,
            supports_multi_draw_indirect,
        ))
    }
}

/// The loader, the instance and the physical device, with the features to
/// enable. Translation of `VULKAN_INTERNAL_PrepareVulkan()`.
fn prepare_vulkan(
    debug_mode: bool,
    prefer_low_power: bool,
    props: &Properties,
) -> Result<(PreparedVulkan, VulkanFeatures)> {
    let (vk_get_instance_proc_addr, global) = load_entry_points()?;

    let mut features = VulkanFeatures::default();

    // Opt out device features (higher compatibility in exchange for reduced functionality)
    let desired = &mut features.desired_vulkan10_device_features;
    desired.sampler_anisotropy = props
        .get_bool(PROP_GPU_DEVICE_CREATE_FEATURE_ANISOTROPY_BOOLEAN)
        .unwrap_or(true) as VkBool32;
    desired.depth_clamp = props
        .get_bool(PROP_GPU_DEVICE_CREATE_FEATURE_DEPTH_CLAMPING_BOOLEAN)
        .unwrap_or(true) as VkBool32;
    desired.shader_clip_distance = props
        .get_bool(PROP_GPU_DEVICE_CREATE_FEATURE_CLIP_DISTANCE_BOOLEAN)
        .unwrap_or(true) as VkBool32;
    desired.draw_indirect_first_instance = props
        .get_bool(PROP_GPU_DEVICE_CREATE_FEATURE_INDIRECT_DRAW_FIRST_INSTANCE_BOOLEAN)
        .unwrap_or(true) as VkBool32;

    // These features have near universal support so they are always enabled
    desired.independent_blend = VK_TRUE;
    desired.sample_rate_shading = VK_TRUE;
    desired.image_cube_array = VK_TRUE;

    // Handle opt-in device features
    add_opt_in_vulkan_options(props, debug_mode, &mut features);

    let require_hardware_acceleration = props
        .get_bool(PROP_GPU_DEVICE_CREATE_VULKAN_REQUIRE_HARDWARE_ACCELERATION_BOOLEAN)
        .unwrap_or(false);

    let (instance, support) = match create_instance(debug_mode, &global, &features) {
        Ok(created) => created,
        Err(e) => {
            crate::log::warn!(Category::Gpu, "Vulkan: Could not create Vulkan instance");
            return Err(e);
        }
    };

    let lookup = |name: &CStr| {
        // SAFETY: the loader's vkGetInstanceProcAddr with the live instance
        // and a NUL-terminated name.
        unsafe { vk_get_instance_proc_addr(instance, name.as_ptr()) }
    };
    // Note (upstream): C doesn't check the instance functions.
    // SAFETY: vkGetInstanceProcAddr returns the functions of the names.
    let inst = match unsafe { InstanceFunctions::load(lookup) } {
        Ok(inst) => inst,
        Err(name) => {
            // FIXME (upstream): the instance leaks when a later step fails.
            return Err(Error::new(format!(
                "vkGetInstanceProcAddr(instance, \"{name}\") failed"
            )));
        }
    };

    let mut prepared = PreparedVulkan {
        vk_get_instance_proc_addr,
        global,
        instance,
        inst,
        debug_mode,
        prefer_low_power,
        require_hardware_acceleration,
        supports_debug_utils: support.debug_utils,
        supports_colorspace: support.colorspace,
        supports_physical_device_properties2: support.physical_device_properties2,
        supports_portability_enumeration: support.portability_enumeration,
        physical_device: null_mut(),
        supports: VulkanExtensions::default(),
        queue_family_index: 0,
        physical_device_properties: VkPhysicalDeviceProperties2::default(),
        physical_device_driver_properties: VkPhysicalDeviceDriverProperties::default(),
        memory_properties: VkPhysicalDeviceMemoryProperties::default(),
    };

    if !prepared.determine_physical_device(&features) {
        crate::log::warn!(
            Category::Gpu,
            "Vulkan: Failed to determine a suitable physical device"
        );
        // FIXME (upstream): the instance leaks.
        return Err(Error::new("Failed to determine a suitable physical device"));
    }
    Ok((prepared, features))
}

/// Whether the Vulkan backend can work. Translation of
/// `VULKAN_PrepareDriver()`.
pub(super) fn prepare_driver(video: &dyn VideoDriver, props: &Properties) -> bool {
    if !props
        .get_bool(PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN)
        .unwrap_or(false)
    {
        return false;
    }

    if !video.implements_vulkan_surfaces() {
        return false;
    }

    if crate::video::vulkan::vulkan_load_library(None).is_err() {
        return false;
    }

    // (HAVE_GPU_OPENXR: OpenXR is not translated.)

    // This needs to be set early for log filtering
    let debug_mode = props
        .get_bool(PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN)
        .unwrap_or(false);

    let prefer_low_power = props
        .get_bool(PROP_GPU_DEVICE_CREATE_PREFERLOWPOWER_BOOLEAN)
        .unwrap_or(false);

    let result = match prepare_vulkan(debug_mode, prefer_low_power, props) {
        Ok((prepared, _)) => {
            // SAFETY: the instance just created, with nothing made from it.
            unsafe { (prepared.inst.destroy_instance)(prepared.instance, null()) };
            true
        }
        Err(_) => false,
    };

    crate::video::vulkan::vulkan_unload_library();

    result
}

/// The driver version string of a physical device (vendor-specific
/// encodings; from `VULKAN_CreateDevice()`).
pub(super) fn driver_version_string(raw_driver_ver: u32, vendor_id: u32) -> String {
    if vendor_id == 0x10de {
        // Nvidia uses 10|8|8|6 encoding.
        format!(
            "{}.{}.{}.{}",
            (raw_driver_ver >> 22) & 0x3ff,
            (raw_driver_ver >> 14) & 0xff,
            (raw_driver_ver >> 6) & 0xff,
            raw_driver_ver & 0x3f
        )
    } else if cfg!(windows) && vendor_id == 0x8086 {
        // Intel uses 18|14 encoding on Windows only.
        format!(
            "{}.{}",
            (raw_driver_ver >> 14) & 0x3ffff,
            raw_driver_ver & 0x3fff
        )
    } else {
        // Assume standard Vulkan 10|10|12 encoding for everything else. AMD and
        // Mesa are known to use this encoding.
        format!(
            "{}.{}.{}",
            (raw_driver_ver >> 22) & 0x3ff,
            (raw_driver_ver >> 12) & 0x3ff,
            raw_driver_ver & 0xfff
        )
    }
}

/// Create the device. Translation of `VULKAN_CreateDevice()`.
pub(super) fn create_device(
    debug_mode: bool,
    prefer_low_power: bool,
    props: &Properties,
) -> Result<VulkanRenderer> {
    let verbose_logs = props
        .get_bool(PROP_GPU_DEVICE_CREATE_VERBOSE_BOOLEAN)
        .unwrap_or(true);

    if crate::video::vulkan::vulkan_load_library(None).is_err() {
        crate::sdl_assert!(!"This should have failed in PrepareDevice first!");
        return Err(Error::new(
            "This should have failed in PrepareDevice first!",
        ));
    }

    // (HAVE_GPU_OPENXR: OpenXR is not translated.)

    let (prepared, mut features) = match prepare_vulkan(debug_mode, prefer_low_power, props) {
        Ok(prepared) => prepared,
        Err(_) => {
            let e = set_string_error(debug_mode, "Failed to initialize Vulkan!");
            crate::video::vulkan::vulkan_unload_library();
            return Err(e);
        }
    };

    let device_props = Properties::new();
    if verbose_logs {
        crate::log::info!(Category::Gpu, "SDL_GPU Driver: Vulkan");
    }

    // Record device name
    let device_name = c_name(&prepared.physical_device_properties.properties.device_name);
    device_props.set(PROP_GPU_DEVICE_NAME_STRING, device_name.as_str())?;
    if verbose_logs {
        crate::log::info!(Category::Gpu, "Vulkan Device: {}", device_name);
    }

    // Record driver version. This is provided as a backup if
    // VK_KHR_driver_properties is not available but as most drivers support it
    // this property should be rarely used.
    //
    // This uses a vendor-specific encoding and it isn't well documented. The
    // vendor ID is the registered PCI ID of the vendor and can be found in
    // online databases.
    let properties = &prepared.physical_device_properties.properties;
    let driver_ver = driver_version_string(properties.driver_version, properties.vendor_id);
    device_props.set(PROP_GPU_DEVICE_DRIVER_VERSION_STRING, driver_ver.as_str())?;
    // Log this only if VK_KHR_driver_properties is not available.

    if prepared.supports.khr_driver_properties {
        // Record driver name and version
        let driver_properties = &prepared.physical_device_driver_properties;
        let driver_name = c_name(&driver_properties.driver_name);
        let driver_info = c_name(&driver_properties.driver_info);
        device_props.set(PROP_GPU_DEVICE_DRIVER_NAME_STRING, driver_name.as_str())?;
        device_props.set(PROP_GPU_DEVICE_DRIVER_INFO_STRING, driver_info.as_str())?;
        if verbose_logs {
            // FIXME: driverInfo can be a multiline string.
            crate::log::info!(
                Category::Gpu,
                "Vulkan Driver: {} {}",
                driver_name,
                driver_info
            );
        }

        // Record conformance level
        if verbose_logs {
            let c = driver_properties.conformance_version;
            let conformance = format!("{}.{}.{}.{}", c.major, c.minor, c.subminor, c.patch);
            crate::log::info!(Category::Gpu, "Vulkan Conformance: {}", conformance);
        }
    } else if verbose_logs {
        crate::log::info!(Category::Gpu, "Vulkan Driver: {}", driver_ver);
    }

    let (
        logical_device,
        dev,
        unified_queue,
        supports_fill_mode_non_solid,
        supports_multi_draw_indirect,
    ) = match prepared.create_logical_device(&mut features) {
        Ok(device) => device,
        Err(_) => {
            // FIXME (upstream): the instance leaks.
            let e = set_string_error(debug_mode, "Failed to create logical device!");
            crate::video::vulkan::vulkan_unload_library();
            return Err(e);
        }
    };

    let min_ubo_alignment = prepared
        .physical_device_properties
        .properties
        .limits
        .min_uniform_buffer_offset_alignment as u32;

    let renderer = VulkanRenderer {
        vk_get_instance_proc_addr: prepared.vk_get_instance_proc_addr,
        global: prepared.global,
        instance: prepared.instance,
        inst: prepared.inst,
        physical_device: prepared.physical_device,
        physical_device_properties: prepared.physical_device_properties,
        physical_device_driver_properties: prepared.physical_device_driver_properties,
        logical_device,
        dev,
        integrated_memory_notification: AtomicBool::new(false),
        out_of_device_local_memory_warning: AtomicBool::new(false),
        outof_bar_memory_warning: AtomicBool::new(false),
        fill_mode_only_warning: AtomicBool::new(false),
        minimum_vk_version: VK_API_VERSION_1_0,
        debug_mode,
        prefer_low_power,
        require_hardware_acceleration: prepared.require_hardware_acceleration,
        props: device_props,
        allowed_frames_in_flight: AtomicU32::new(2),
        supports: prepared.supports,
        supports_debug_utils: prepared.supports_debug_utils,
        supports_colorspace: prepared.supports_colorspace,
        supports_physical_device_properties2: prepared.supports_physical_device_properties2,
        supports_portability_enumeration: prepared.supports_portability_enumeration,
        supports_fill_mode_non_solid,
        supports_multi_draw_indirect,
        // Memory Allocator
        memory_allocator: ReentrantMutex::new(RefCell::new(MemoryAllocator::new())),
        memory_properties: prepared.memory_properties,
        queue_family_index: prepared.queue_family_index,
        unified_queue,
        submit_lock: Mutex::new(()),
        dispose: Mutex::new(PendingDestroys::default()),
        // Initialize caches
        render_pass_hash_table: Mutex::new(HashMap::new()),
        framebuffer_hash_table: Mutex::new(HashMap::new()),
        graphics_pipeline_resource_layout_hash_table: Mutex::new(HashMap::new()),
        compute_pipeline_resource_layout_hash_table: Mutex::new(HashMap::new()),
        descriptor_set_layout_hash_table: Mutex::new(HashMap::new()),
        uniform_buffer_pool: Mutex::new(Vec::with_capacity(32)),
        descriptor_set_cache_pool: Mutex::new(Vec::with_capacity(8)),
        layout_resource_id: AtomicU32::new(0),
        // Device limits
        min_ubo_alignment,
        defrag_lock: RwLock::new(()),
        // Defrag state
        defrag_in_progress: AtomicBool::new(false),
    };

    // Create uniform buffer pool

    // Note (upstream): C doesn't check these creations (a failed one is a
    // NULL dereference); the device fails here.
    for _ in 0..32 {
        match renderer.create_uniform_buffer(UNIFORM_BUFFER_SIZE) {
            Ok(uniform_buffer) => super::lock(&renderer.uniform_buffer_pool).push(uniform_buffer),
            Err(e) => {
                let mut renderer = renderer;
                crate::gpu::sysgpu::GpuDriver::destroy(&mut renderer);
                return Err(e);
            }
        }
    }

    Ok(renderer)
}

impl VulkanRenderer {
    /// Destroy everything the device made, the device and the instance.
    /// Translation of `VULKAN_DestroyDevice()`.
    pub(super) fn destroy_device(&mut self) {
        let _ = self.wait_internal();

        // part 2: release the claimed windows (VULKAN_ReleaseWindow) and
        // wait again.

        // part 2: free the submitted command buffers.

        for uniform_buffer in std::mem::take(&mut *super::lock(&self.uniform_buffer_pool)) {
            let buffer = super::lock(&uniform_buffer.state).buffer.clone();
            self.destroy_buffer(&buffer);
        }

        for descriptor_set_cache in
            std::mem::take(&mut *super::lock(&self.descriptor_set_cache_pool))
        {
            self.destroy_descriptor_set_cache(descriptor_set_cache);
        }

        // part 2: destroy the fence pool's fences and the command pools
        // (commandPoolHashTable).

        for (_, render_pass) in std::mem::take(&mut *super::lock(&self.render_pass_hash_table)) {
            // SAFETY: a render pass of the device, no longer used.
            unsafe { (self.dev.destroy_render_pass)(self.logical_device, render_pass, null()) };
        }

        {
            // FIXME (upstream): the framebuffers are released (to be
            // destroyed by a PerformPendingDestroys that never comes), so
            // they outlive the device.
            let framebuffers = std::mem::take(&mut *super::lock(&self.framebuffer_hash_table));
            let mut dispose = super::lock(&self.dispose);
            for (_, framebuffer) in framebuffers {
                self.release_framebuffer(&mut dispose, framebuffer);
            }
        }

        for (_, layout) in std::mem::take(&mut *super::lock(
            &self.graphics_pipeline_resource_layout_hash_table,
        )) {
            self.destroy_pipeline_layout(layout.pipeline_layout);
        }

        for (_, layout) in std::mem::take(&mut *super::lock(
            &self.compute_pipeline_resource_layout_hash_table,
        )) {
            self.destroy_pipeline_layout(layout.pipeline_layout);
        }

        for (_, layout) in std::mem::take(&mut *super::lock(&self.descriptor_set_layout_hash_table))
        {
            self.destroy_descriptor_set_layout(&layout);
        }

        for i in 0..VK_MAX_MEMORY_TYPES {
            let count = self.memory_allocator.lock().borrow().allocation_count(i);
            for j in (0..count).rev() {
                let used_regions = {
                    let allocator = self.memory_allocator.lock();
                    let a = allocator.borrow();
                    a.used_regions(a.allocation_key(i, j))
                };
                for used_region in used_regions.iter().rev() {
                    self.remove_memory_used_region(used_region);
                }

                self.deallocate_memory(i, j);
            }
        }

        // SAFETY: the device, with nothing of it left in use.
        unsafe { (self.dev.destroy_device)(self.logical_device, null()) };
        // SAFETY: the instance, with nothing of it left.
        unsafe { (self.inst.destroy_instance)(self.instance, null()) };

        crate::video::vulkan::vulkan_unload_library();
    }
}
