// Rust translation of the device instantiation of
// src/gpu/vulkan/SDL_gpu_vulkan.c from Simple DirectMedia Layer: the
// extension and feature checks, the instance, the choice of the physical
// device, the logical device (each through the OpenXR runtime for a
// device made with OpenXR), and VULKAN_PrepareDriver(), the OpenXR entry
// points, VULKAN_CreateDevice() and VULKAN_DestroyDevice().
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Device instantiation. Upstream fills a zeroed `VulkanRenderer` step by
//! step (`VULKAN_INTERNAL_PrepareVulkan()` is shared by `PrepareDriver`,
//! which destroys what it made, and `CreateDevice`); here the steps fill a
//! [`PreparedVulkan`], which `CreateDevice` turns into the renderer once
//! the logical device exists.
//!
//! A device made with OpenXR (`HAVE_GPU_OPENXR`, on every target, as
//! upstream's CMake builds it) has its instance, physical device and
//! logical device made by the runtime (`XR_KHR_vulkan_enable2`:
//! `xrCreateVulkanInstanceKHR`, `xrGetVulkanGraphicsDevice2KHR`,
//! `xrCreateVulkanDeviceKHR`), and the OpenXR entry points
//! (`CreateXRSession`...) are here, between PrepareDriver and CreateDevice,
//! where upstream has them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Arc, Mutex, RwLock};

use super::memory::MemoryAllocator;
use super::resources::PendingDestroys;
use super::resources::{
    get_texture_subresource_index, ContainerRef, TextureContainer, TextureContainerState,
    TextureSubresource, VulkanTexture, VulkanTextureUsageMode,
};
use super::tables::vk_error_messages;
use super::tables::{sdl_to_vk_texture_format, swizzle_for_sdl_format, SDL_TO_VK_TEXTURE_FORMAT};
use super::vkfuncs::{DeviceFunctions, GlobalFunctions, InstanceFunctions};
use super::{set_string_error, vulkan_error, VulkanRenderer};
use crate::error::{Error, Result};
use crate::gpu::sysgpu::{BackendObject, UNIFORM_BUFFER_SIZE};
use crate::gpu::xr::gpu_openxr::{
    c_name as xr_name, destroy_unreturned_xr_instance, gpu_init_openxr,
};
use crate::gpu::xr::openxrdyn::{self, XrInstancePfns};
use crate::gpu::xr::sys::*;
use crate::gpu::xr::XrError;
use crate::gpu::{
    SampleCount, TextureCreateInfo, TextureFormat, TextureType, TextureUsageFlags,
    VulkanFeatureStructure, VulkanOptions, PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_FEATURE_ANISOTROPY_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_FEATURE_CLIP_DISTANCE_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_FEATURE_DEPTH_CLAMPING_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_FEATURE_INDIRECT_DRAW_FIRST_INSTANCE_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_PREFERLOWPOWER_BOOLEAN, PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_VERBOSE_BOOLEAN, PROP_GPU_DEVICE_CREATE_VULKAN_OPTIONS_POINTER,
    PROP_GPU_DEVICE_CREATE_VULKAN_REQUIRE_HARDWARE_ACCELERATION_BOOLEAN,
    PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN, PROP_GPU_DEVICE_CREATE_XR_FORM_FACTOR_NUMBER,
    PROP_GPU_DEVICE_CREATE_XR_VERSION_NUMBER, PROP_GPU_DEVICE_DRIVER_INFO_STRING,
    PROP_GPU_DEVICE_DRIVER_NAME_STRING, PROP_GPU_DEVICE_DRIVER_VERSION_STRING,
    PROP_GPU_DEVICE_NAME_STRING,
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

    // OpenXR
    /// a non-null instance also states this vk device was created by OpenXR
    pub(super) xr_instance: XrInstance,
    pub(super) xr_system_id: XrSystemId,

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
///
/// Note (upstream): C never releases the reference to the loader this
/// takes (each PrepareDriver and CreateDevice takes one more, so the
/// loader is never unloaded); here it's released before returning, the
/// caller's own reference (PrepareDriver's, or the device's) keeping the
/// loader loaded.
fn load_entry_points() -> Result<(PfnVkGetInstanceProcAddr, GlobalFunctions)> {
    // Required for MoltenVK support
    let _ = crate::stdlib::setenv_unsafe("MVK_CONFIG_FULL_IMAGE_VIEW_SWIZZLE", "1", true);

    // Load Vulkan entry points
    if let Err(e) = crate::video::vulkan::vulkan_load_library(None) {
        crate::log::warn!(Category::Gpu, "Vulkan: SDL_Vulkan_LoadLibrary failed!");
        return Err(e);
    }

    let result = load_global_functions();
    crate::video::vulkan::vulkan_unload_library();
    result
}

/// The loader's `vkGetInstanceProcAddr` and the global functions (in
/// `VULKAN_INTERNAL_LoadEntryPoints()`).
fn load_global_functions() -> Result<(PfnVkGetInstanceProcAddr, GlobalFunctions)> {
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
/// the optional instance extensions it has. With an OpenXR instance
/// (`xr_instance`), the runtime makes it (`xrCreateVulkanInstanceKHR`),
/// with the loader's `vkGetInstanceProcAddr`.
fn create_instance(
    debug_mode: bool,
    vk_get_instance_proc_addr: PfnVkGetInstanceProcAddr,
    global: &GlobalFunctions,
    features: &VulkanFeatures,
    xr_instance: XrInstance,
    xr_system_id: XrSystemId,
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

    let mut instance = null_mut();
    let vulkan_result = if xr_instance != XR_NULL_HANDLE {
        // SAFETY: the function's type.
        let (_, xr_create_vulkan_instance_khr) = unsafe {
            openxrdyn::xr_get_instance_proc_addr::<PfnXrCreateVulkanInstanceKHR>(
                xr_instance,
                "xrCreateVulkanInstanceKHR",
            )
        };
        let Some(xr_create_vulkan_instance_khr) = xr_create_vulkan_instance_khr else {
            crate::log::debug!(Category::Gpu, "Failed to get xrCreateVulkanInstanceKHR");
            return Err(Error::new("Failed to get xrCreateVulkanInstanceKHR"));
        };

        let xr_create_info = XrVulkanInstanceCreateInfoKHR {
            ty: XR_TYPE_VULKAN_INSTANCE_CREATE_INFO_KHR,
            vulkan_create_info: (&create_info as *const VkInstanceCreateInfo).cast(),
            system_id: xr_system_id,
            // (SDL_Vulkan_GetVkGetInstanceProcAddr())
            pfn_get_instance_proc_addr: Some(vk_get_instance_proc_addr),
            ..Default::default()
        };
        // Note (upstream): upstream's `vulkanResult` is uninitialized, and
        // logged below when the runtime's call fails without writing it
        // (an indeterminate value); here it is VK_SUCCESS then.
        let mut vulkan_result = VK_SUCCESS;
        // SAFETY: the runtime's function, with a valid create info whose
        // structures and strings outlive the call, and out-pointers.
        let xr_result = unsafe {
            xr_create_vulkan_instance_khr(
                xr_instance,
                &xr_create_info,
                &mut instance,
                &mut vulkan_result,
            )
        };
        if xr_result != XrResult::SUCCESS {
            crate::log::debug!(
                Category::Gpu,
                "Failed to create vulkan instance, reason {}, {}",
                xr_result.0,
                vulkan_result
            );
            return Err(Error::new("Failed to create vulkan instance"));
        }
        vulkan_result
    } else {
        // SAFETY: a valid create info whose strings outlive the call.
        unsafe { (global.create_instance)(&create_info, null(), &mut instance) }
    };

    if vulkan_result != VK_SUCCESS {
        return Err(vulkan_error(debug_mode, vulkan_result, "vkCreateInstance"));
    }
    count_instance(1);

    Ok((instance, support))
}

/// The instances made and not destroyed yet, which the tests check.
#[cfg(test)]
pub(super) static LIVE_INSTANCES: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(0);

/// Count an instance made (1) or destroyed (-1), for the tests.
fn count_instance(delta: isize) {
    #[cfg(test)]
    LIVE_INSTANCES.fetch_add(delta, std::sync::atomic::Ordering::SeqCst);
    let _ = delta;
}

/// Destroy an instance with nothing made from it left.
///
/// # Safety
///
/// `destroy` is the instance's `vkDestroyInstance`.
unsafe fn destroy_instance(
    destroy: unsafe extern "system" fn(VkInstance, Alloc),
    instance: VkInstance,
) {
    // SAFETY: the caller's instance and its function.
    unsafe { destroy(instance, null()) };
    count_instance(-1);
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

    /// Choose the physical device: the OpenXR runtime's, with OpenXR.
    /// Translation of `VULKAN_INTERNAL_DeterminePhysicalDevice()`.
    fn determine_physical_device(&mut self, features: &VulkanFeatures) -> bool {
        // When XR is enabled, let the OpenXR runtime choose the physical device
        if self.xr_instance != XR_NULL_HANDLE {
            // SAFETY: the function's type.
            let (xr_result, xr_get_vulkan_graphics_device2_khr) = unsafe {
                openxrdyn::xr_get_instance_proc_addr::<PfnXrGetVulkanGraphicsDevice2KHR>(
                    self.xr_instance,
                    "xrGetVulkanGraphicsDevice2KHR",
                )
            };
            let Some(xr_get_vulkan_graphics_device2_khr) = xr_get_vulkan_graphics_device2_khr
            else {
                crate::log::error!(
                    Category::Gpu,
                    "Failed to get xrGetVulkanGraphicsDevice2KHR, result: {}",
                    xr_result.0
                );
                return false;
            };

            let graphics_device_get_info = XrVulkanGraphicsDeviceGetInfoKHR {
                ty: XR_TYPE_VULKAN_GRAPHICS_DEVICE_GET_INFO_KHR,
                system_id: self.xr_system_id,
                vulkan_instance: self.instance,
                ..Default::default()
            };

            // SAFETY: the runtime's function, with the info and an
            // out-pointer.
            let xr_result = unsafe {
                xr_get_vulkan_graphics_device2_khr(
                    self.xr_instance,
                    &graphics_device_get_info,
                    &mut self.physical_device,
                )
            };
            if xr_result != XrResult::SUCCESS {
                crate::log::error!(
                    Category::Gpu,
                    "xrGetVulkanGraphicsDevice2KHR failed, result: {}",
                    xr_result.0
                );
                return false;
            }

            // Verify the XR-chosen device is suitable
            let Some((xr_physical_device_extensions, queue_family_index)) =
                self.is_device_suitable(features, self.physical_device)
            else {
                crate::log::error!(
                    Category::Gpu,
                    "The physical device chosen by the OpenXR runtime is not suitable"
                );
                return false;
            };

            self.supports = xr_physical_device_extensions;
            self.queue_family_index = queue_family_index;
        } else if !self.choose_physical_device(features) {
            return false;
        }

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

    /// Choose the best suitable device of the instance (upstream's
    /// `else` branch of `VULKAN_INTERNAL_DeterminePhysicalDevice()`).
    fn choose_physical_device(&mut self, features: &VulkanFeatures) -> bool {
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

        let mut logical_device = null_mut();
        let vulkan_result = if self.xr_instance != XR_NULL_HANDLE {
            // SAFETY: the function's type.
            let (_, xr_create_vulkan_device_khr) = unsafe {
                openxrdyn::xr_get_instance_proc_addr::<PfnXrCreateVulkanDeviceKHR>(
                    self.xr_instance,
                    "xrCreateVulkanDeviceKHR",
                )
            };
            let Some(xr_create_vulkan_device_khr) = xr_create_vulkan_device_khr else {
                crate::log::debug!(Category::Gpu, "Failed to get xrCreateVulkanDeviceKHR");
                return Err(Error::new("Failed to get xrCreateVulkanDeviceKHR"));
            };

            let xr_device_create_info = XrVulkanDeviceCreateInfoKHR {
                ty: XR_TYPE_VULKAN_DEVICE_CREATE_INFO_KHR,
                vulkan_create_info: (&device_create_info as *const VkDeviceCreateInfo).cast(),
                system_id: self.xr_system_id,
                vulkan_physical_device: self.physical_device,
                // (SDL_Vulkan_GetVkGetInstanceProcAddr())
                pfn_get_instance_proc_addr: Some(self.vk_get_instance_proc_addr),
                ..Default::default()
            };
            // Note (upstream): uninitialized in C, and logged below when
            // the runtime's call fails without writing it (as in
            // create_instance); VK_SUCCESS here.
            let mut vulkan_result = VK_SUCCESS;
            // SAFETY: the runtime's function, with a valid create info whose
            // chain and arrays outlive the call, and out-pointers.
            let xr_result = unsafe {
                xr_create_vulkan_device_khr(
                    self.xr_instance,
                    &xr_device_create_info,
                    &mut logical_device,
                    &mut vulkan_result,
                )
            };
            if xr_result != XrResult::SUCCESS {
                crate::log::error!(
                    Category::Gpu,
                    "Failed to create OpenXR Vulkan logical device, result {}, {}",
                    xr_result.0,
                    vulkan_result
                );
                return Err(Error::new("Failed to create OpenXR Vulkan logical device"));
            }
            vulkan_result
        } else {
            // SAFETY: a valid create info whose chain and arrays outlive the
            // call.
            unsafe {
                (self.inst.create_device)(
                    self.physical_device,
                    &device_create_info,
                    null(),
                    &mut logical_device,
                )
            }
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
/// enable; through the OpenXR runtime with an OpenXR instance
/// (`xr_instance`, the renderer's `xrInstance`). Translation of
/// `VULKAN_INTERNAL_PrepareVulkan()`.
fn prepare_vulkan(
    debug_mode: bool,
    prefer_low_power: bool,
    props: &Properties,
    xr_instance: XrInstance,
    xr_system_id: XrSystemId,
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

    let (instance, support) = match create_instance(
        debug_mode,
        vk_get_instance_proc_addr,
        &global,
        &features,
        xr_instance,
        xr_system_id,
    ) {
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
            // Note (upstream): the instance leaks when a later step fails;
            // here it's destroyed (with its vkDestroyInstance, if that's
            // found).
            if let Some(destroy) = lookup(c"vkDestroyInstance") {
                // SAFETY: vkGetInstanceProcAddr's vkDestroyInstance.
                unsafe {
                    destroy_instance(
                        std::mem::transmute::<
                            PfnVkVoidFunction,
                            unsafe extern "system" fn(VkInstance, Alloc),
                        >(destroy),
                        instance,
                    )
                };
            }
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
        xr_instance,
        xr_system_id,
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
        // Note (upstream): the instance leaks; here it's destroyed.
        // SAFETY: the instance just created, with nothing made from it.
        unsafe { destroy_instance(prepared.inst.destroy_instance, prepared.instance) };
        return Err(Error::new("Failed to determine a suitable physical device"));
    }
    Ok((prepared, features))
}

/// The runtime's Vulkan extension (`XR_KHR_vulkan_enable2`), if it has it.
/// Translation of `VULKAN_INTERNAL_SearchForOpenXrGpuExtension()`.
fn search_for_openxr_gpu_extension() -> Option<XrExtensionProperties> {
    // (a loader without the function: see openxrdyn::get_sym)
    let xr_enumerate_instance_extension_properties =
        openxrdyn::globals().xrEnumerateInstanceExtensionProperties?;

    let mut extension_count: u32 = 0;
    // SAFETY: a count query.
    let result = unsafe {
        xr_enumerate_instance_extension_properties(null(), 0, &mut extension_count, null_mut())
    };
    if result != XrResult::SUCCESS {
        return None;
    }

    let mut extension_properties = vec![
        XrExtensionProperties {
            ty: XR_TYPE_EXTENSION_PROPERTIES,
            ..Default::default()
        };
        extension_count as usize
    ];

    // SAFETY: an array of extension_count structures.
    let result = unsafe {
        xr_enumerate_instance_extension_properties(
            null(),
            extension_count,
            &mut extension_count,
            extension_properties.as_mut_ptr(),
        )
    };
    if result != XrResult::SUCCESS {
        return None;
    }

    for extension in extension_properties.iter().take(extension_count as usize) {
        // NOTE: as generally recommended, we support KHR_vulkan_enable2 *only*
        // see https://fredemmott.com/blog/2024/11/25/best-practices-for-openxr-api-layers.html
        if xr_name(&extension.extension_name) == XR_KHR_VULKAN_ENABLE2_EXTENSION_NAME {
            crate::log::info!(
                Category::Gpu,
                "Found {} extension",
                XR_KHR_VULKAN_ENABLE2_EXTENSION_NAME
            );

            return Some(*extension);
        }
    }

    None
}

/// The runtime's minimum Vulkan API version for the system (an
/// `XrVersion`), or the failed call's result. Translation of
/// `VULKAN_INTERNAL_GetXrMinimumVulkanApiVersion()`.
fn get_xr_minimum_vulkan_api_version(
    instance: XrInstance,
    system_id: XrSystemId,
) -> std::result::Result<XrVersion, XrResult> {
    // SAFETY: the function's type.
    let (xr_result, xr_get_vulkan_graphics_requirements2_khr) = unsafe {
        openxrdyn::xr_get_instance_proc_addr::<PfnXrGetVulkanGraphicsRequirements2KHR>(
            instance,
            "xrGetVulkanGraphicsRequirements2KHR",
        )
    };
    let Some(xr_get_vulkan_graphics_requirements2_khr) = xr_get_vulkan_graphics_requirements2_khr
    else {
        crate::log::debug!(
            Category::Gpu,
            "Failed to get xrGetVulkanGraphicsRequirements2KHR"
        );
        return Err(xr_result);
    };

    let mut graphics_requirements_vulkan = XrGraphicsRequirementsVulkanKHR {
        ty: XR_TYPE_GRAPHICS_REQUIREMENTS_VULKAN2_KHR,
        ..Default::default()
    };
    // SAFETY: the runtime's function, with an out-structure.
    let xr_result = unsafe {
        xr_get_vulkan_graphics_requirements2_khr(
            instance,
            system_id,
            &mut graphics_requirements_vulkan,
        )
    };
    if xr_result != XrResult::SUCCESS {
        crate::log::debug!(
            Category::Gpu,
            "Failed to get vulkan graphics requirements, got OpenXR error {}",
            xr_result.0
        );
        return Err(xr_result);
    }

    Ok(graphics_requirements_vulkan.min_api_version_supported)
}

/// `XR_VERSION_MAJOR/MINOR/PATCH(version)` as a Vulkan version's parts.
fn xr_version_parts(version: XrVersion) -> (u32, u32, u32) {
    (
        xr_version_major(version) as u32,
        xr_version_minor(version) as u32,
        xr_version_patch(version) as u32,
    )
}

/// Whether the Vulkan backend can work. Translation of
/// `VULKAN_PrepareDriver()`.
///
/// With OpenXR, a throwaway OpenXR instance (named "SDL", with the
/// runtime's Vulkan extension only) checks the runtime's system, its Vulkan
/// requirements, and the instance and physical device it gives.
///
/// Note (upstream): the errors it sets ("Failed to find a suitable OpenXR
/// GPU extension.", "Failed to get the minimum supported Vulkan API
/// version.") are replaced by the backend selection's ("No supported
/// SDL_GPU backend found!") before the application can see them; here a
/// PrepareDriver has no error to return.
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

    let mut instance_pfns: Option<XrInstancePfns> = None;
    let mut xr_instance: XrInstance = XR_NULL_HANDLE;
    let mut xr_system_id: XrSystemId = XR_NULL_SYSTEM_ID;
    let xr = props
        .get_bool(PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN)
        .unwrap_or(false);

    /* On Android/Quest, don't test XR in PrepareDriver. The Quest OpenXR runtime
     * can't handle having its instance created and destroyed during preparation
     * and then recreated during device creation. Just return true for XR mode
     * and let CreateDevice do the real work. */
    #[cfg(target_os = "android")]
    if xr {
        crate::video::vulkan::vulkan_unload_library();
        return true;
    }

    if xr {
        if let Err(e) = openxrdyn::load_library() {
            crate::log::warn!(
                Category::Gpu,
                "Vulkan: Failed to load OpenXR: {}",
                e.message()
            );
            crate::video::vulkan::vulkan_unload_library();
            return false;
        }

        let Some(gpu_extension) = search_for_openxr_gpu_extension() else {
            // (SDL_SetError("Failed to find a suitable OpenXR GPU
            // extension."): see the Note above)
            crate::video::vulkan::vulkan_unload_library();
            openxrdyn::unload_library();
            return false;
        };

        let extension_name = gpu_extension.extension_name.as_ptr();
        let mut application_info = XrApplicationInfo {
            api_version: props
                .get_number(PROP_GPU_DEVICE_CREATE_XR_VERSION_NUMBER)
                .map(|v| v as XrVersion)
                .unwrap_or(XR_API_VERSION_1_0),
            ..Default::default()
        };
        for (d, &c) in application_info.application_name.iter_mut().zip(b"SDL") {
            *d = c as c_char;
        }
        let create_info = XrInstanceCreateInfo {
            ty: XR_TYPE_INSTANCE_CREATE_INFO,
            application_info,
            enabled_extension_count: 1,
            enabled_extension_names: &extension_name,
            ..Default::default()
        };
        // (a loader without the function: see openxrdyn::get_sym)
        let xr_result = match openxrdyn::globals().xrCreateInstance {
            // SAFETY: the create info and its array, alive for the call.
            Some(xr_create_instance) => unsafe {
                xr_create_instance(&create_info, &mut xr_instance)
            },
            None => XrResult::ERROR_FUNCTION_UNSUPPORTED,
        };
        if xr_result != XrResult::SUCCESS {
            crate::log::debug!(Category::Gpu, "Failed to create OpenXR instance");
            crate::video::vulkan::vulkan_unload_library();
            openxrdyn::unload_library();
            return false;
        }

        let Some(pfns) = openxrdyn::load_instance_symbols(xr_instance) else {
            crate::log::debug!(
                Category::Gpu,
                "Failed to load needed OpenXR instance symbols"
            );
            // FIXME (upstream): the instance is left alive (without the
            // instance's functions there's no xrDestroyInstance to call);
            // so it is here.
            crate::video::vulkan::vulkan_unload_library();
            openxrdyn::unload_library();
            return false;
        };
        let pfns = instance_pfns.insert(pfns);

        let system_get_info = XrSystemGetInfo {
            ty: XR_TYPE_SYSTEM_GET_INFO,
            form_factor: props
                .get_number(PROP_GPU_DEVICE_CREATE_XR_FORM_FACTOR_NUMBER)
                .map(|f| f as XrFormFactor)
                .unwrap_or(XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY),
            ..Default::default()
        };
        // SAFETY: the instance's function, the info and an out-pointer.
        let xr_result =
            unsafe { (pfns.xrGetSystem)(xr_instance, &system_get_info, &mut xr_system_id) };
        if xr_result != XrResult::SUCCESS {
            crate::log::debug!(Category::Gpu, "Failed to get OpenXR system");
            // SAFETY: the instance made above.
            unsafe { (pfns.xrDestroyInstance)(xr_instance) };
            crate::video::vulkan::vulkan_unload_library();
            openxrdyn::unload_library();
            return false;
        }
    }

    // This needs to be set early for log filtering
    let debug_mode = props
        .get_bool(PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN)
        .unwrap_or(false);

    let prefer_low_power = props
        .get_bool(PROP_GPU_DEVICE_CREATE_PREFERLOWPOWER_BOOLEAN)
        .unwrap_or(false);

    if let Some(pfns) = &instance_pfns {
        match get_xr_minimum_vulkan_api_version(xr_instance, xr_system_id) {
            Ok(minimum_vk_version_xr) => {
                let (major, minor, patch) = xr_version_parts(minimum_vk_version_xr);
                // FIXME (upstream): the renderer's minimumVkVersion is
                // computed and never used (nor is CreateDevice's): the
                // instance is made for Vulkan 1.0 (or the Vulkan options'
                // version) whatever the runtime's minimum.
                let _minimum_vk_version = vk_make_api_version(0, major, minor, patch);
            }
            Err(_) => {
                // (SDL_SetError("Failed to get the minimum supported Vulkan
                // API version."): see the Note above)
                // SAFETY: the instance made above.
                unsafe { (pfns.xrDestroyInstance)(xr_instance) };
                crate::video::vulkan::vulkan_unload_library();
                openxrdyn::unload_library();
                return false;
            }
        }
    }

    let result = match prepare_vulkan(
        debug_mode,
        prefer_low_power,
        props,
        xr_instance,
        xr_system_id,
    ) {
        Ok((prepared, _)) => {
            // SAFETY: the instance just created, with nothing made from it.
            unsafe { destroy_instance(prepared.inst.destroy_instance, prepared.instance) };
            true
        }
        Err(_) => false,
    };

    if let Some(pfns) = instance_pfns {
        // SAFETY: the instance made above, whose Vulkan instance is gone.
        unsafe { (pfns.xrDestroyInstance)(xr_instance) };
        // (SDL_free(instancePfns))
        openxrdyn::unload_library();
    }

    crate::video::vulkan::vulkan_unload_library();

    result
}

impl VulkanRenderer {
    /// The OpenXR instance's functions (`renderer->xr`), or the error of a
    /// device made without OpenXR.
    ///
    /// Note (upstream): the OpenXR entry points use `renderer->xr`, which
    /// is NULL on a device made without OpenXR, so C dereferences NULL
    /// there (SIGSEGV); here they fail with `XR_ERROR_FUNCTION_UNSUPPORTED`
    /// (the formats with no list), as the backends without OpenXR do.
    fn xr_pfns(&self) -> Result<&XrInstancePfns> {
        self.xr
            .as_ref()
            .ok_or_else(|| Error::new("The GPU device was not created with OpenXR enabled"))
    }

    /// [`VulkanRenderer::xr_pfns`] for the entry points that return an
    /// `XrResult`.
    fn xr_pfns_or_unsupported(&self) -> std::result::Result<&XrInstancePfns, XrError> {
        self.xr_pfns()
            .map_err(|e| XrError::new(XrResult::ERROR_FUNCTION_UNSUPPORTED, e))
    }

    /// Destroy an OpenXR swapchain and the textures of its images.
    /// Translation of `VULKAN_DestroyXRSwapchain()`.
    ///
    /// Note (upstream): a texture that isn't an OpenXR swapchain's gives
    /// "Invalid GPU Texture handle." once the textures before it are
    /// destroyed, and the swapchain is kept; and each container's array of
    /// textures is never freed (here it is, with the container). An array
    /// shorter than the swapchain's images is read past its end; here the
    /// destruction stops at its end.
    pub(super) fn destroy_xr_swapchain(
        &self,
        swapchain: XrSwapchain,
        swapchain_images: &[BackendObject],
    ) -> std::result::Result<(), XrError> {
        let xr = self.xr_pfns_or_unsupported()?;

        let _ = self.wait_internal();

        let mut swapchain_count: u32 = 0;
        // SAFETY: the instance's function, a count query.
        let result = unsafe {
            (xr.xrEnumerateSwapchainImages)(swapchain, 0, &mut swapchain_count, null_mut())
        };
        if result != XrResult::SUCCESS {
            return Err(XrError::from_result(result));
        }

        // We always want to destroy the swapchain images, so don't early return if xrDestroySwapchain fails for some reason
        for image in swapchain_images.iter().take(swapchain_count as usize) {
            let container = super::object::<TextureContainer>(image);

            if !container.externally_managed {
                return Err(XrError::new(
                    XrResult::ERROR_HANDLE_INVALID,
                    Error::new("Invalid GPU Texture handle."),
                ));
            }

            let active_texture = super::lock(&container.state).active_texture.clone();
            self.destroy_texture(&mut super::lock(&self.dispose), &active_texture);

            // Free the container now that it's unused
            // (the front end drops its handles next)
        }

        // (SDL_free(swapchainImages): the front end's)

        // SAFETY: the instance's function, with the swapchain.
        let result = unsafe { (xr.xrDestroySwapchain)(swapchain) };
        if result.failed() {
            return Err(XrError::from_result(result));
        }
        Ok(())
    }

    /// The session's swapchain formats that have a texture format, in the
    /// runtime's order (the first texture format of each). Translation of
    /// `VULKAN_GetXRSwapchainFormats()`.
    ///
    /// Note (upstream): when the runtime's call fails, C returns `NULL`
    /// with no error set; and it frees the runtime's array twice and its
    /// own never (harmless where `SDL_stack_alloc` is `alloca`). Here the
    /// error says which call failed, and the arrays are owned.
    pub(super) fn get_xr_swapchain_formats(
        &self,
        session: XrSession,
    ) -> Result<Vec<TextureFormat>> {
        let xr = self.xr_pfns()?;

        let mut num_supported_formats: u32 = 0;
        // SAFETY: the instance's function, a count query.
        let result = unsafe {
            (xr.xrEnumerateSwapchainFormats)(session, 0, &mut num_supported_formats, null_mut())
        };
        if result != XrResult::SUCCESS {
            return Err(Error::new(format!(
                "xrEnumerateSwapchainFormats failed ({result})"
            )));
        }
        let mut supported_formats = vec![0i64; num_supported_formats as usize];
        // SAFETY: an array of num_supported_formats formats.
        let result = unsafe {
            (xr.xrEnumerateSwapchainFormats)(
                session,
                num_supported_formats,
                &mut num_supported_formats,
                supported_formats.as_mut_ptr(),
            )
        };
        if result != XrResult::SUCCESS {
            return Err(Error::new(format!(
                "xrEnumerateSwapchainFormats failed ({result})"
            )));
        }

        let mut sdl_formats = Vec::with_capacity(num_supported_formats as usize);

        // Iterate over all formats the runtime supports
        for &supported_format in supported_formats
            .iter()
            .take(num_supported_formats as usize)
        {
            // Iterate over all formats we support
            if let Some(j) = SDL_TO_VK_TEXTURE_FORMAT
                .iter()
                .position(|&vk_format| i64::from(vk_format) == supported_format)
            {
                // Add the format match we found, linearly. The output order should match the order of the runtime.
                sdl_formats.push(TextureFormat(j as u32));
            }
        }

        if sdl_formats.is_empty() {
            return Err(Error::new(
                "Failed to find a swapchain format supported by both OpenXR and SDL",
            ));
        }

        Ok(sdl_formats)
    }

    /// Create an OpenXR swapchain in a texture format, and a texture of
    /// each of its images (a container of one texture, never cycled, whose
    /// `VkImage` is the runtime's). Translation of
    /// `VULKAN_CreateXRSwapchain()`.
    ///
    /// FIXME (upstream): when a call after `xrCreateSwapchain` fails, the
    /// swapchain is left alive (C also leaks the images' array and the
    /// containers made so far, which are owned here).
    ///
    /// Note (upstream): the textures' usage is COLOR_TARGET whatever the
    /// swapchain's usage flags (their render target views are made only
    /// with `XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT`), they have no full
    /// view (they can't be sampled), and their type is 2D whatever the
    /// array size.
    ///
    /// FIXME (upstream): so every image is transitioned to the color
    /// attachment layout, which Vulkan allows only for images made with
    /// `VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT`: a swapchain without
    /// `XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT`, on a runtime whose images
    /// have exactly the usage asked for (the repository's test runtime;
    /// Monado adds that usage itself), gets a validation error an image
    /// (`VUID-VkImageMemoryBarrier-oldLayout-01208`). Kept, as upstream's
    /// C does it.
    pub(super) fn create_xr_swapchain(
        &self,
        session: XrSession,
        old_create_info: &XrSwapchainCreateInfo,
        format: TextureFormat,
    ) -> std::result::Result<(XrSwapchain, Vec<(BackendObject, TextureCreateInfo)>), XrError> {
        let xr = self.xr_pfns_or_unsupported()?;

        let mut create_info = *old_create_info;
        create_info.format = i64::from(sdl_to_vk_texture_format(format));

        let mut swapchain: XrSwapchain = XR_NULL_HANDLE;
        // SAFETY: the instance's function, with the create info (the
        // application's chain is its to keep valid) and an out-pointer.
        let result = unsafe { (xr.xrCreateSwapchain)(session, &create_info, &mut swapchain) };
        if result != XrResult::SUCCESS {
            return Err(XrError::from_result(result));
        }

        let mut swapchain_image_count: u32 = 0;
        // SAFETY: the instance's function, a count query.
        let result = unsafe {
            (xr.xrEnumerateSwapchainImages)(swapchain, 0, &mut swapchain_image_count, null_mut())
        };
        if result != XrResult::SUCCESS {
            return Err(XrError::from_result(result));
        }

        let mut swapchain_images = vec![
            XrSwapchainImageVulkanKHR {
                ty: XR_TYPE_SWAPCHAIN_IMAGE_VULKAN2_KHR,
                ..Default::default()
            };
            swapchain_image_count as usize
        ];

        // SAFETY: an array of swapchain_image_count Vulkan images, whose
        // header the structures begin with.
        let result = unsafe {
            (xr.xrEnumerateSwapchainImages)(
                swapchain,
                swapchain_image_count,
                &mut swapchain_image_count,
                swapchain_images.as_mut_ptr().cast(),
            )
        };
        if result != XrResult::SUCCESS {
            return Err(XrError::from_result(result));
        }

        let vk_format = sdl_to_vk_texture_format(format);
        let mut texture_containers = Vec::with_capacity(swapchain_image_count as usize);

        for swapchain_image in swapchain_images.iter().take(swapchain_image_count as usize) {
            let vk_image = swapchain_image.image;

            // Note (upstream): C leaves the texture's levelCount and
            // layerCount 0, so the transition to its default usage below is
            // a barrier over no subresources (which the validation layer
            // reports: VUID-VkImageSubresourceRange-levelCount-01720 and
            // -layerCount-01721), and its container unset; here they're the
            // swapchain's mip count and array size, and the container is set
            // once it's made.
            let mut texture = VulkanTexture {
                container: Mutex::new(None),
                used_region: None,
                image: vk_image,
                full_view: VK_NULL_HANDLE,
                swizzle: swizzle_for_sdl_format(format),
                aspect_flags: VK_IMAGE_ASPECT_COLOR_BIT,
                depth: 1,
                level_count: create_info.mip_count,
                layer_count: create_info.array_size,
                ty: TextureType::Texture2D,
                usage: TextureUsageFlags::COLOR_TARGET,
                subresources: Vec::new(),
                marked_for_destroy: AtomicBool::new(false),
                externally_managed: true,
                reference_count: std::sync::atomic::AtomicI32::new(0),
            };

            let subresource_count = create_info.array_size * create_info.mip_count;
            texture.subresources = (0..subresource_count)
                .map(|_| TextureSubresource::default())
                .collect();

            for i in 0..create_info.array_size {
                for j in 0..create_info.mip_count {
                    let subresource_index =
                        get_texture_subresource_index(j, i, create_info.mip_count) as usize;

                    if create_info.usage_flags & XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT != 0 {
                        match self.create_render_target_view(
                            &texture,
                            i,
                            j,
                            vk_format,
                            texture.swizzle,
                        ) {
                            Ok(view) => texture.subresources[subresource_index]
                                .render_target_views
                                .push(view),
                            Err(_) => {
                                self.destroy_texture(&mut super::lock(&self.dispose), &texture);
                                return Err(XrError::new(
                                    XrResult::ERROR_RUNTIME_FAILURE,
                                    Error::new("Failed to create render target view"),
                                ));
                            }
                        }
                    }

                    texture.subresources[subresource_index].layer = i;
                    texture.subresources[subresource_index].level = j;
                }
            }
            let texture = Arc::new(texture);

            // Transition to the default barrier state
            // Note (upstream): C goes on with a NULL command buffer when none
            // can be acquired, and ignores the submission's failure; both
            // fail here.
            let mut barrier_command_buffer = self
                .acquire_command_buffer_internal()
                .map_err(|e| XrError::new(XrResult::ERROR_RUNTIME_FAILURE, e))?;
            self.texture_transition_to_default_usage(
                &mut barrier_command_buffer,
                VulkanTextureUsageMode::Uninitialized,
                &texture,
            );
            barrier_command_buffer.track_texture(&texture);
            self.submit_internal(barrier_command_buffer)
                .map_err(|e| XrError::new(XrResult::ERROR_RUNTIME_FAILURE, e))?;

            let info = TextureCreateInfo {
                texture_type: TextureType::Texture2D,
                format,
                usage: TextureUsageFlags::COLOR_TARGET,
                width: create_info.width,
                height: create_info.height,
                layer_count_or_depth: create_info.array_size,
                num_levels: create_info.mip_count,
                sample_count: SampleCount::One,
                props: None,
            };
            let container = Arc::new(TextureContainer {
                info: info.clone(),
                state: Mutex::new(TextureContainerState {
                    active_texture: texture.clone(),
                    textures: vec![texture.clone()],
                    debug_name: None,
                }),
                can_be_cycled: false,
                externally_managed: true,
            });
            texture.set_container(Some(ContainerRef {
                container: Arc::downgrade(&container),
                index: 0,
            }));
            texture_containers.push((BackendObject(container), info));
        }

        Ok((swapchain, texture_containers))
    }

    /// Create an OpenXR session of the device's system, with the device's
    /// graphics binding before the application's chain of structures.
    /// Translation of `VULKAN_CreateXRSession()`.
    pub(super) fn create_xr_session(
        &self,
        createinfo: &XrSessionCreateInfo,
    ) -> std::result::Result<XrSession, XrError> {
        let xr = self.xr_pfns_or_unsupported()?;

        // Copy out the existing next ptr so that we can append it to the end of the chain we create
        let current_next_ptr = createinfo.next;

        // KHR_vulkan_enable and KHR_vulkan_enable2 share this structure, so we don't need to change any logic here to handle both
        let graphics_binding = XrGraphicsBindingVulkanKHR {
            ty: XR_TYPE_GRAPHICS_BINDING_VULKAN_KHR,
            instance: self.instance,
            physical_device: self.physical_device,
            device: self.logical_device,
            queue_family_index: self.queue_family_index,
            queue_index: 0, // we only ever have one queue, so hardcode queue index 0
            next: current_next_ptr,
        };

        let mut session_create_info = *createinfo;
        session_create_info.system_id = self.xr_system_id;
        session_create_info.next = (&graphics_binding as *const XrGraphicsBindingVulkanKHR).cast();

        let mut session: XrSession = XR_NULL_HANDLE;
        // SAFETY: the instance's function, with the create info and its
        // chain (the application's part is its to keep valid), and an
        // out-pointer.
        let result =
            unsafe { (xr.xrCreateSession)(self.xr_instance, &session_create_info, &mut session) };
        if result.failed() {
            return Err(XrError::from_result(result));
        }
        Ok(session)
    }
}

/// For the tests' skip reasons: what the physical devices lack of the
/// Vulkan 1.0 features the backend asks for by default (the ones
/// `VULKAN_INTERNAL_IsDeviceSuitable()` checks first), as `NAME has no
/// FEATURE, ...` for each device, or `None` when a device has them all (or
/// there is no Vulkan to ask).
#[cfg(test)]
fn missing_device_features() -> Option<String> {
    crate::video::vulkan::vulkan_load_library(None).ok()?;
    let result = (|| {
        let (vk_get_instance_proc_addr, global) = load_global_functions().ok()?;
        let (instance, _) = create_instance(
            false,
            vk_get_instance_proc_addr,
            &global,
            &VulkanFeatures::default(),
            XR_NULL_HANDLE,
            XR_NULL_SYSTEM_ID,
        )
        .ok()?;
        let lookup = |name: &CStr| {
            // SAFETY: the loader's vkGetInstanceProcAddr with the live
            // instance and a NUL-terminated name.
            unsafe { vk_get_instance_proc_addr(instance, name.as_ptr()) }
        };
        // SAFETY: vkGetInstanceProcAddr returns the functions of the names.
        let inst = unsafe { InstanceFunctions::load(lookup) }.ok()?;
        let mut count = 0;
        // SAFETY: the instance, with a null array to query the count, then
        // room for the devices.
        let mut devices = unsafe {
            (inst.enumerate_physical_devices)(instance, &mut count, null_mut());
            let mut devices = vec![null_mut(); count as usize];
            (inst.enumerate_physical_devices)(instance, &mut count, devices.as_mut_ptr());
            devices
        };
        devices.truncate(count as usize);
        let mut lacking = Vec::new();
        let mut suitable = devices.is_empty();
        for device in devices {
            let mut f = VkPhysicalDeviceFeatures::default();
            let mut properties = VkPhysicalDeviceProperties::default();
            // SAFETY: a physical device of the instance.
            unsafe {
                (inst.get_physical_device_features)(device, &mut f);
                (inst.get_physical_device_properties)(device, &mut properties);
            }
            let missing: Vec<&str> = [
                ("independentBlend", f.independent_blend),
                ("imageCubeArray", f.image_cube_array),
                ("depthClamp", f.depth_clamp),
                ("shaderClipDistance", f.shader_clip_distance),
                ("drawIndirectFirstInstance", f.draw_indirect_first_instance),
                ("sampleRateShading", f.sample_rate_shading),
                ("samplerAnisotropy", f.sampler_anisotropy),
            ]
            .into_iter()
            .filter(|&(_, supported)| supported == 0)
            .map(|(name, _)| name)
            .collect();
            if missing.is_empty() {
                suitable = true;
            } else {
                lacking.push(format!(
                    "{} has no {}",
                    c_name(&properties.device_name),
                    missing.join(", ")
                ));
            }
        }
        // SAFETY: the instance just created, with nothing made from it.
        unsafe { destroy_instance(inst.destroy_instance, instance) };
        (!suitable).then(|| lacking.join("; "))
    })();
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

/// Create the device; with OpenXR, the instance and system first
/// (`SDL_OPENXR_INTERNAL_GPUInitOpenXR()`), then the Vulkan objects
/// through the runtime. Translation of `VULKAN_CreateDevice()`.
///
/// Note (upstream): upstream refuses a device made with OpenXR without
/// its out pointers for the instance and the system; here they're the
/// renderer's (`Device::xr_instance`, `Device::xr_system_id`). And C sets
/// no error when the runtime has no Vulkan extension, when the instance
/// or the system can't be had, or when the runtime's requirements can't be
/// read (`SDL_GetError()` says what failed before); here the error says
/// which step failed.
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

    let mut minimum_vk_version = VK_API_VERSION_1_0;

    let xr = props
        .get_bool(PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN)
        .unwrap_or(false);
    let mut xr_init = None;

    if xr {
        if let Err(e) = openxrdyn::load_library() {
            crate::sdl_assert!(!"This should have failed in PrepareDevice first!");
            crate::video::vulkan::vulkan_unload_library();
            return Err(e);
        }

        let Some(gpu_extension) = search_for_openxr_gpu_extension() else {
            crate::log::debug!(
                Category::Gpu,
                "Failed to find a compatible OpenXR vulkan extension"
            );
            openxrdyn::unload_library();
            crate::video::vulkan::vulkan_unload_library();
            return Err(Error::new(
                "Failed to find a compatible OpenXR vulkan extension",
            ));
        };

        let Some(init) = gpu_init_openxr(debug_mode, &gpu_extension, props) else {
            crate::log::debug!(Category::Gpu, "Failed to init OpenXR");
            openxrdyn::unload_library();
            crate::video::vulkan::vulkan_unload_library();
            return Err(Error::new("Failed to init OpenXR"));
        };

        let Ok(minimum_vulkan_api_version) =
            get_xr_minimum_vulkan_api_version(init.instance, init.system_id)
        else {
            crate::log::debug!(Category::Gpu, "Failed to get OpenXR graphics requirements");
            // SAFETY: the instance GPUInitOpenXR made.
            unsafe { (init.xr.xrDestroyInstance)(init.instance) };
            openxrdyn::unload_library();
            // (SDL_free(renderer->xr))
            crate::video::vulkan::vulkan_unload_library();
            return Err(Error::new("Failed to get OpenXR graphics requirements"));
        };

        let (major, minor, patch) = xr_version_parts(minimum_vulkan_api_version);
        minimum_vk_version = vk_make_version(major, minor, patch);
        xr_init = Some(init);
    }

    let (xr_instance, xr_system_id) = xr_init
        .as_ref()
        .map_or((XR_NULL_HANDLE, XR_NULL_SYSTEM_ID), |init| {
            (init.instance, init.system_id)
        });

    let (prepared, mut features) = match prepare_vulkan(
        debug_mode,
        prefer_low_power,
        props,
        xr_instance,
        xr_system_id,
    ) {
        Ok(prepared) => prepared,
        Err(_) => {
            let e = set_string_error(debug_mode, "Failed to initialize Vulkan!");
            if let Some(init) = &xr_init {
                // SAFETY: the instance GPUInitOpenXR made, with no Vulkan
                // instance of it left (prepare_vulkan destroys its own).
                unsafe { (init.xr.xrDestroyInstance)(init.instance) };
                openxrdyn::unload_library();
                // (SDL_free(renderer->xr))
            }
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
            // Note (upstream): the instance leaks; here it's destroyed.
            // SAFETY: the instance, with nothing made from it left.
            unsafe { destroy_instance(prepared.inst.destroy_instance, prepared.instance) };
            let e = set_string_error(debug_mode, "Failed to create logical device!");
            // Note (upstream): C leaves the OpenXR instance (and the
            // loader's reference) to the application, which has it through
            // the out pointer; here the application never sees it, so it's
            // destroyed (the reference is kept, as upstream keeps it).
            destroy_unreturned_xr_instance(xr_init.as_ref());
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
        minimum_vk_version,
        xr_instance,
        xr_system_id,
        xr: xr_init.as_ref().map(|init| init.xr),
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
        claimed_windows: Mutex::new(Vec::with_capacity(1)),
        unified_queue,
        submit_lock: Mutex::new(Vec::with_capacity(16)),
        fence_pool: Mutex::new(Vec::with_capacity(4)),
        command_pools: Mutex::new(HashMap::new()),
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
                // (as for the logical device's failure)
                destroy_unreturned_xr_instance(xr_init.as_ref());
                return Err(e);
            }
        }
    }

    Ok(renderer)
}

impl VulkanRenderer {
    /// Destroy everything the device made, the device and the instance.
    /// Translation of `VULKAN_DestroyDevice()`.
    ///
    /// Note (upstream): a device made with OpenXR keeps its reference to
    /// the OpenXR loader (C never releases it, nor frees `renderer->xr`,
    /// which is owned here), so that the application's OpenXR functions
    /// stay valid; and its OpenXR instance is the application's to destroy
    /// after the device (the test program's comment says the device
    /// destroys it, which it doesn't).
    pub(super) fn destroy_device(&mut self) {
        let _ = self.wait_internal();

        self.release_claimed_windows();

        let _ = self.wait_internal();

        // (the submitted command buffers were cleaned by the wait)

        for uniform_buffer in std::mem::take(&mut *super::lock(&self.uniform_buffer_pool)) {
            let buffer = super::lock(&uniform_buffer.state).buffer.clone();
            self.destroy_buffer(&buffer);
        }

        for descriptor_set_cache in
            std::mem::take(&mut *super::lock(&self.descriptor_set_cache_pool))
        {
            self.destroy_descriptor_set_cache(descriptor_set_cache);
        }

        for fence in std::mem::take(&mut *super::lock(&self.fence_pool)) {
            // SAFETY: a fence of the device nothing waits on.
            unsafe { (self.dev.destroy_fence)(self.logical_device, fence.fence, null()) };
        }

        for (_, command_pool) in std::mem::take(&mut *super::lock(&self.command_pools)) {
            // (the hash table's destroy callback)
            self.destroy_command_pool(command_pool);
        }

        for (_, render_pass) in std::mem::take(&mut *super::lock(&self.render_pass_hash_table)) {
            // SAFETY: a render pass of the device, no longer used.
            unsafe { (self.dev.destroy_render_pass)(self.logical_device, render_pass, null()) };
        }

        // Note (upstream): the framebuffers are released (to be destroyed by
        // a PerformPendingDestroys that never comes), so they outlive the
        // device; here they're destroyed (the GPU is idle). (Only those of
        // textures never released can be left: the front end releases every
        // texture before the device.)
        for (_, framebuffer) in std::mem::take(&mut *super::lock(&self.framebuffer_hash_table)) {
            self.destroy_framebuffer(&framebuffer);
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
        unsafe { destroy_instance(self.inst.destroy_instance, self.instance) };

        crate::video::vulkan::vulkan_unload_library();
    }
}

/// A test's skip reason for a failure to make a Vulkan device (or what
/// needs one): `what`, with what the physical devices lack when none has
/// the features the backend asks for (as the simulators' Metal devices
/// don't, through MoltenVK).
#[cfg(test)]
pub(crate) fn no_device_reason(what: std::fmt::Arguments<'_>) -> String {
    match missing_device_features() {
        Some(why) => format!("{what}: {why}"),
        None => what.to_string(),
    }
}
