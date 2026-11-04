// Rust translation of src/gpu/xr/SDL_gpu_openxr.c and SDL_gpu_openxr.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The OpenXR instance and system a GPU backend's device is made with
//! ([`gpu_init_openxr`]): the application's API layers and extensions from
//! the `SDL.gpu.device.create.xr.*` properties, the backend's graphics
//! extension, and in debug mode the core validation layer when the loader
//! has it.

#![allow(non_snake_case)]
// (upstream's names)
// (for the backends' OpenXR paths: none is built on Emscripten, and Direct3D
// 12's isn't translated yet)
#![allow(dead_code)]

use std::ffi::{c_char, CStr, CString};

use super::openxrdyn::{self, XrInstancePfns};
use super::sys::*;
use crate::log::Category;
use crate::properties::Properties;

use super::super::{
    PROP_GPU_DEVICE_CREATE_XR_APPLICATION_NAME_STRING,
    PROP_GPU_DEVICE_CREATE_XR_APPLICATION_VERSION_NUMBER,
    PROP_GPU_DEVICE_CREATE_XR_ENGINE_NAME_STRING, PROP_GPU_DEVICE_CREATE_XR_ENGINE_VERSION_NUMBER,
    PROP_GPU_DEVICE_CREATE_XR_EXTENSION_COUNT_NUMBER,
    PROP_GPU_DEVICE_CREATE_XR_EXTENSION_NAMES_POINTER,
    PROP_GPU_DEVICE_CREATE_XR_FORM_FACTOR_NUMBER, PROP_GPU_DEVICE_CREATE_XR_LAYER_COUNT_NUMBER,
    PROP_GPU_DEVICE_CREATE_XR_LAYER_NAMES_POINTER, PROP_GPU_DEVICE_CREATE_XR_VERSION_NUMBER,
};

const VALIDATION_LAYER_API_NAME: &str = "XR_APILAYER_LUNARG_core_validation";

/// What [`gpu_init_openxr`] made: the instance, its system and its
/// functions (upstream's `*instance`, `*systemId` and `*xr`).
#[derive(Debug)]
pub(crate) struct GpuOpenXr {
    pub(crate) instance: XrInstance,
    pub(crate) system_id: XrSystemId,
    pub(crate) xr: XrInstancePfns,
}

/// The text of a C string array (an `XrApiLayerProperties`'s or
/// `XrExtensionProperties`'s name).
pub(crate) fn c_name(name: &[c_char]) -> String {
    // SAFETY: c_char and u8 have the same layout.
    let bytes = unsafe { std::slice::from_raw_parts(name.as_ptr().cast::<u8>(), name.len()) };
    CStr::from_bytes_until_nul(bytes)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| String::from_utf8_lossy(bytes).into_owned())
}

/// `SDL_strlcpy()` of `src` into a fixed C string array.
fn strlcpy(dst: &mut [c_char], src: &str) {
    let bytes = src.as_bytes();
    let n = bytes.len().min(dst.len().saturating_sub(1));
    for (d, &b) in dst.iter_mut().zip(&bytes[..n]) {
        *d = b as c_char;
    }
    if let Some(end) = dst.get_mut(n) {
        *end = 0;
    }
}

/// On Android, the OpenXR loader is initialized by SDL_OpenXR_LoadLibrary()
/// in SDL_openxrdyn.c which must be called before this. That function
/// handles the complex initialization using direct SDL_LoadFunction calls
/// to avoid issues with xrGetInstanceProcAddr from runtime negotiation not
/// supporting pre-instance calls. This stub is kept for API compatibility.
/// Translation of `SDL_OPENXR_INTERNAL_InitializeAndroidLoader()`.
#[cfg(target_os = "android")]
fn initialize_android_loader() -> bool {
    // The loader should already be initialized by SDL_OpenXR_LoadLibrary().
    // We just verify that xrGetInstanceProcAddr is available.
    if openxrdyn::globals().xrGetInstanceProcAddr.is_none() {
        crate::log::error!(
            Category::Gpu,
            "xrGetInstanceProcAddr is NULL - SDL_OpenXR_LoadLibrary was not called first"
        );
        return false;
    }
    crate::log::info!(
        Category::Gpu,
        "Android OpenXR loader verified (was initialized by SDL_OpenXR_LoadLibrary)"
    );
    true
}

/// Translation of `SDL_OPENXR_INTERNAL_ValidationLayerAvailable()`.
fn validation_layer_available() -> bool {
    #[cfg(target_os = "android")]
    {
        // On Android/Quest, the xrGetInstanceProcAddr obtained through
        // runtime negotiation crashes when used for pre-instance global
        // functions. Skip validation layer check.
        false
    }
    #[cfg(not(target_os = "android"))]
    {
        // (a loader without the function: see openxrdyn::get_sym)
        let Some(xrEnumerateApiLayerProperties) =
            openxrdyn::globals().xrEnumerateApiLayerProperties
        else {
            return false;
        };

        let mut apiLayerCount: u32 = 0;
        // SAFETY: a count query.
        if unsafe { xrEnumerateApiLayerProperties(0, &mut apiLayerCount, std::ptr::null_mut()) }
            .failed()
        {
            return false;
        }

        if apiLayerCount == 0 {
            return false;
        }

        // FIXME (upstream): the structures' type isn't set to
        // XR_TYPE_API_LAYER_PROPERTIES, which the Khronos loader checks
        // (Ubuntu 24.04's 1.0.20: "VUID-XrApiLayerProperties-type-type:
        // unknown type in api_layer_properties"), so with any layer
        // installed this second call fails: nothing is logged and the
        // validation layer is never enabled in debug mode. Kept, as
        // upstream's C does it.
        let mut apiLayerProperties = vec![XrApiLayerProperties::default(); apiLayerCount as usize];
        // SAFETY: an array of apiLayerCount structures.
        if unsafe {
            xrEnumerateApiLayerProperties(
                apiLayerCount,
                &mut apiLayerCount,
                apiLayerProperties.as_mut_ptr(),
            )
        }
        .failed()
        {
            return false;
        }

        let mut found = false;
        for apiLayer in apiLayerProperties.iter().take(apiLayerCount as usize) {
            let name = c_name(&apiLayer.layer_name);
            crate::log::info!(Category::Gpu, "api layer available: {}", name);
            if name == VALIDATION_LAYER_API_NAME {
                found = true;
                break;
            }
        }

        found
    }
}

/// The first `count` names of a names property (`SDL_PROP_GPU_DEVICE_CREATE_XR_*_NAMES_POINTER`,
/// a `Vec<String>` set with [`Properties::set_any`]).
///
/// Note (upstream): upstream copies `count` pointers from the property's
/// array, past its end when the count is larger (or from `NULL` when
/// there's none); here there are at most as many names as the vector
/// holds.
fn names(props: &Properties, count_name: &str, names_name: &str) -> Vec<CString> {
    let count = props.get_number(count_name).unwrap_or(0).max(0) as u32 as usize;
    let Some(names) = props.get_any::<Vec<String>>(names_name) else {
        return Vec::new();
    };
    names
        .iter()
        .take(count)
        .map(|n| CString::new(n.as_str()).unwrap_or_default())
        .collect()
}

/// Translation of `SDL_OPENXR_INTERNAL_GPUInitOpenXR()`: the instance, the
/// system and the instance's functions, or `None` (upstream's `false`).
///
/// Note (upstream): it is typed `XrResult` but returns `true` and `false`
/// (failure is `XR_SUCCESS`); its callers test it with `!`, which makes it
/// work, but its Android loader check returns `XR_ERROR_INITIALIZATION_FAILED`,
/// which they take for success. Here every failure is `None`. And when the
/// instance's functions can't be loaded the instance is left alive (its
/// own note: "we can't actually destroy the created OpenXR instance here,
/// as we only get that function pointer by loading the instance symbols");
/// so it is here.
pub(crate) fn gpu_init_openxr(
    debugMode: bool,
    gpuExtension: &XrExtensionProperties,
    props: &Properties,
) -> Option<GpuOpenXr> {
    // Android requires loader initialization before any other XR calls
    #[cfg(target_os = "android")]
    if !initialize_android_loader() {
        crate::log::debug!(Category::Gpu, "Failed to initialize Android OpenXR loader");
        return None;
    }

    let validationLayersAvailable = validation_layer_available();

    let userApiLayerNames = names(
        props,
        PROP_GPU_DEVICE_CREATE_XR_LAYER_COUNT_NUMBER,
        PROP_GPU_DEVICE_CREATE_XR_LAYER_NAMES_POINTER,
    );
    let userApiLayerCount = userApiLayerNames.len() as u32;

    let userExtensionNames = names(
        props,
        PROP_GPU_DEVICE_CREATE_XR_EXTENSION_COUNT_NUMBER,
        PROP_GPU_DEVICE_CREATE_XR_EXTENSION_NAMES_POINTER,
    );
    let userExtensionCount = userExtensionNames.len() as u32;

    // allocate enough space for the validation layer + the user's api layers
    let validationLayer = CString::new(VALIDATION_LAYER_API_NAME).expect("no NUL");
    let mut apiLayerNames: Vec<*const c_char> =
        userApiLayerNames.iter().map(|n| n.as_ptr()).collect();
    apiLayerNames.push(validationLayer.as_ptr());

    // On Android, we need an extra extension for android_create_instance
    #[cfg(target_os = "android")]
    let platformExtensionCount: u32 = 2; // GPU extension + Android create instance
    #[cfg(not(target_os = "android"))]
    let platformExtensionCount: u32 = 1; // GPU extension only

    let mut extensionNames: Vec<*const c_char> =
        userExtensionNames.iter().map(|n| n.as_ptr()).collect();
    extensionNames.push(gpuExtension.extension_name.as_ptr());
    #[cfg(target_os = "android")]
    let androidCreateInstance =
        CString::new(XR_KHR_ANDROID_CREATE_INSTANCE_EXTENSION_NAME).expect("no NUL");
    #[cfg(target_os = "android")]
    extensionNames.push(androidCreateInstance.as_ptr());

    let mut xrInstanceCreateInfo = XrInstanceCreateInfo {
        ty: XR_TYPE_INSTANCE_CREATE_INFO,
        ..Default::default()
    };
    xrInstanceCreateInfo.application_info.api_version = props
        .get_number(PROP_GPU_DEVICE_CREATE_XR_VERSION_NUMBER)
        .map(|v| v as XrVersion)
        .unwrap_or(XR_API_VERSION_1_0);
    xrInstanceCreateInfo.enabled_api_layer_count = userApiLayerCount
        + if debugMode && validationLayersAvailable {
            1
        } else {
            0
        }; // in debug mode, we enable the validation layer
    xrInstanceCreateInfo.enabled_api_layer_names = apiLayerNames.as_ptr();
    xrInstanceCreateInfo.enabled_extension_count = userExtensionCount + platformExtensionCount;
    xrInstanceCreateInfo.enabled_extension_names = extensionNames.as_ptr();

    // Get JNI environment and JavaVM for Android instance creation
    #[cfg(target_os = "android")]
    let instanceCreateInfoAndroid = {
        let vm = crate::core::android::env::get_env()
            .and_then(|env| env.get_java_vm())
            .map_or(std::ptr::null_mut(), |vm| vm.as_ptr().cast());
        let activity = crate::core::android::sdl_android::get_android_activity()
            .map_or(std::ptr::null_mut(), |a| a.as_ptr());
        XrInstanceCreateInfoAndroidKHR {
            ty: XR_TYPE_INSTANCE_CREATE_INFO_ANDROID_KHR,
            next: std::ptr::null(),
            application_vm: vm,
            application_activity: activity,
        }
    };
    #[cfg(target_os = "android")]
    {
        xrInstanceCreateInfo.next =
            &instanceCreateInfoAndroid as *const XrInstanceCreateInfoAndroidKHR as *const _;
    }

    let applicationVersion = props
        .get_number(PROP_GPU_DEVICE_CREATE_XR_APPLICATION_VERSION_NUMBER)
        .unwrap_or(0) as u32;
    let applicationName = props
        .get_string(PROP_GPU_DEVICE_CREATE_XR_APPLICATION_NAME_STRING)
        .or_else(|| crate::init::app_metadata_property(crate::init::AppMetadata::Name))
        .unwrap_or_default();

    strlcpy(
        &mut xrInstanceCreateInfo.application_info.application_name,
        &applicationName,
    );
    xrInstanceCreateInfo.application_info.application_version = applicationVersion;

    let engineName = props
        .get_string(PROP_GPU_DEVICE_CREATE_XR_ENGINE_NAME_STRING)
        .unwrap_or_else(|| "SDLGPU".to_owned());
    let engineVersion = props
        .get_number(PROP_GPU_DEVICE_CREATE_XR_ENGINE_VERSION_NUMBER)
        .unwrap_or(crate::version::Version::CURRENT.to_number() as i64)
        as u32;

    strlcpy(
        &mut xrInstanceCreateInfo.application_info.engine_name,
        &engineName,
    );
    xrInstanceCreateInfo.application_info.engine_version = engineVersion;

    let mut instance: XrInstance = XR_NULL_HANDLE;
    // (a loader without the function: see openxrdyn::get_sym)
    let created = match openxrdyn::globals().xrCreateInstance {
        // SAFETY: the create info and its arrays, alive for the call.
        Some(xrCreateInstance) => unsafe { xrCreateInstance(&xrInstanceCreateInfo, &mut instance) },
        None => XrResult::ERROR_FUNCTION_UNSUPPORTED,
    };
    if created != XrResult::SUCCESS {
        crate::log::debug!(Category::Gpu, "Failed to create OpenXR instance");
        return None;
    }

    let Some(xr) = openxrdyn::load_instance_symbols(instance) else {
        crate::log::debug!(
            Category::Gpu,
            "Failed to load required OpenXR instance symbols"
        );
        /* NOTE: we can't actually destroy the created OpenXR instance here,
        as we only get that function pointer by loading the instance symbols...
        let's just hope that doesn't happen. */
        return None;
    };

    let systemGetInfo = XrSystemGetInfo {
        ty: XR_TYPE_SYSTEM_GET_INFO,
        form_factor: props
            .get_number(PROP_GPU_DEVICE_CREATE_XR_FORM_FACTOR_NUMBER)
            .map(|f| f as XrFormFactor)
            .unwrap_or(XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY),
        ..Default::default()
    };
    let mut systemId: XrSystemId = XR_NULL_SYSTEM_ID;
    // SAFETY: the instance's function, the info and an out-pointer.
    if unsafe { (xr.xrGetSystem)(instance, &systemGetInfo, &mut systemId) } != XrResult::SUCCESS {
        crate::log::debug!(Category::Gpu, "Failed to get OpenXR system");
        // SAFETY: the instance made above.
        unsafe { (xr.xrDestroyInstance)(instance) };
        return None;
    }

    Some(GpuOpenXr {
        instance,
        system_id: systemId,
        xr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strlcpy_truncates_and_terminates() {
        let mut a = [1 as c_char; 4];
        strlcpy(&mut a, "abcdef");
        assert_eq!(c_name(&a), "abc");
        strlcpy(&mut a, "");
        assert_eq!(a[0], 0);
        assert_eq!(c_name(&[b'x' as c_char, b'y' as c_char]), "xy");
    }

    #[test]
    fn names_take_the_count() {
        let props = Properties::new();
        let n = |p: &Properties| names(p, "count", "names");
        assert!(n(&props).is_empty());
        props
            .set_any("names", vec!["A".to_owned(), "B".to_owned()])
            .unwrap();
        assert!(n(&props).is_empty());
        props.set("count", 1i64).unwrap();
        assert_eq!(n(&props), vec![CString::new("A").unwrap()]);
        props.set("count", 5i64).unwrap();
        assert_eq!(n(&props).len(), 2);
    }
}
