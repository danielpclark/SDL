// sdl3-openxr-test-runtime: an OpenXR runtime for the tests of the sdl3
// crate's GPU OpenXR functions, and nothing else.
// Part of the repository's own test code (not a translation of SDL, nor of
// any OpenXR runtime: written from the OpenXR 1.0 specification and its
// loader's runtime interface); zlib-licensed like the repository, see
// LICENSE.txt.

//! # sdl3-openxr-test-runtime
//!
//! A test-only OpenXR runtime, a shared library (`cdylib`) that the
//! repository's tests load in either of two ways:
//!
//! * through the Khronos OpenXR loader, as any runtime: a runtime manifest
//!   (`XR_RUNTIME_JSON`, or `HKLM\SOFTWARE\Khronos\OpenXR\1\ActiveRuntime`
//!   on Windows) naming the library, which exports
//!   `xrNegotiateLoaderRuntimeInterface`;
//! * in the loader's place, by `SDL_OPENXR_LIBRARY` (the sdl3 crate's
//!   `hints::OPENXR_LIBRARY`): the library also exports the four global
//!   functions a loader does (`xrGetInstanceProcAddr`,
//!   `xrEnumerateApiLayerProperties`, `xrEnumerateInstanceExtensionProperties`,
//!   `xrCreateInstance`). The loader's specification asks a runtime that
//!   exports such names to be linked with `-Bsymbolic` on Linux (so that
//!   the loader's symbols of the same names never stand in for its own):
//!   the build script does it; and the runtime never calls nor hands out
//!   its exported symbols, only the functions behind them.
//!
//! It implements what SDL calls, and what the tests' application side calls
//! around it, of OpenXR 1.0 and `XR_KHR_vulkan_enable2` (its only extension):
//! instances, the system, the Vulkan instance, physical device and device
//! made for the application, sessions and their states, swapchains of
//! Vulkan images made on the application's device, reference spaces, views
//! and the frame loop (see `runtime.rs` for its fixed choices: formats,
//! image counts, states). The other core functions aren't there:
//! `xrGetInstanceProcAddr` answers `XR_ERROR_FUNCTION_UNSUPPORTED` for
//! them. Every Vulkan function comes through the application's
//! `vkGetInstanceProcAddr`: the runtime loads no library.
//!
//! Everything is deterministic (handles count from 1, the time from a fixed
//! start), and any call can be made to fail through the environment
//! (`SDL3_OPENXR_TEST_RUNTIME_FAIL`, see `fail.rs`).
//!
//! The library is built on every target the workspace builds, from `std`
//! alone; only Linux's tests use it so far.

#![allow(clippy::missing_safety_doc)] // (the functions' contracts are OpenXR's)

mod fail;
mod runtime;
mod sys;
mod vulkan;

use std::ffi::{c_char, CStr};

use sys::*;

/// A function as `PFN_xrVoidFunction`.
macro_rules! void_fn {
    ($f:expr) => {
        // SAFETY: a function pointer as another function pointer type, to be
        // cast back by the caller to its own.
        Some(unsafe { std::mem::transmute::<*const (), PfnXrVoidFunction>($f as *const ()) })
    };
}

/// The functions served for an instance: their names, whether they need
/// `XR_KHR_vulkan_enable2`, and the functions.
fn function(name: &[u8]) -> Option<(bool, Option<PfnXrVoidFunction>)> {
    use runtime::*;
    Some(match name {
        b"xrGetInstanceProcAddr" => (false, void_fn!(get_instance_proc_addr)),
        b"xrEnumerateApiLayerProperties" => (false, void_fn!(enumerate_api_layer_properties)),
        b"xrEnumerateInstanceExtensionProperties" => {
            (false, void_fn!(enumerate_instance_extension_properties))
        }
        b"xrCreateInstance" => (false, void_fn!(create_instance)),
        b"xrDestroyInstance" => (false, void_fn!(destroy_instance)),
        b"xrGetSystem" => (false, void_fn!(get_system)),
        b"xrPollEvent" => (false, void_fn!(poll_event)),
        b"xrCreateSession" => (false, void_fn!(create_session)),
        b"xrDestroySession" => (false, void_fn!(destroy_session)),
        b"xrBeginSession" => (false, void_fn!(begin_session)),
        b"xrEndSession" => (false, void_fn!(end_session)),
        b"xrRequestExitSession" => (false, void_fn!(request_exit_session)),
        b"xrEnumerateSwapchainFormats" => (false, void_fn!(enumerate_swapchain_formats)),
        b"xrCreateSwapchain" => (false, void_fn!(create_swapchain)),
        b"xrDestroySwapchain" => (false, void_fn!(destroy_swapchain)),
        b"xrEnumerateSwapchainImages" => (false, void_fn!(enumerate_swapchain_images)),
        b"xrAcquireSwapchainImage" => (false, void_fn!(acquire_swapchain_image)),
        b"xrWaitSwapchainImage" => (false, void_fn!(wait_swapchain_image)),
        b"xrReleaseSwapchainImage" => (false, void_fn!(release_swapchain_image)),
        b"xrCreateReferenceSpace" => (false, void_fn!(create_reference_space)),
        b"xrDestroySpace" => (false, void_fn!(destroy_space)),
        b"xrLocateViews" => (false, void_fn!(locate_views)),
        b"xrWaitFrame" => (false, void_fn!(wait_frame)),
        b"xrBeginFrame" => (false, void_fn!(begin_frame)),
        b"xrEndFrame" => (false, void_fn!(end_frame)),
        b"xrGetVulkanGraphicsRequirements2KHR" => {
            (true, void_fn!(get_vulkan_graphics_requirements2))
        }
        b"xrCreateVulkanInstanceKHR" => (true, void_fn!(create_vulkan_instance)),
        b"xrGetVulkanGraphicsDevice2KHR" => (true, void_fn!(get_vulkan_graphics_device2)),
        b"xrCreateVulkanDeviceKHR" => (true, void_fn!(create_vulkan_device)),
        _ => return None,
    })
}

/// `xrGetInstanceProcAddr`.
unsafe extern "system" fn get_instance_proc_addr(
    instance: XrInstance,
    name: *const c_char,
    function_out: *mut Option<PfnXrVoidFunction>,
) -> XrResult {
    if name.is_null() || function_out.is_null() {
        return XR_ERROR_VALIDATION_FAILURE;
    }
    // SAFETY: checked above; the application's NUL-terminated name.
    let name = unsafe { CStr::from_ptr(name) }.to_bytes();
    // SAFETY: checked above.
    unsafe { *function_out = None };
    if let Some(result) = fail::xr_call(&format!(
        "xrGetInstanceProcAddr:{}",
        String::from_utf8_lossy(name)
    )) {
        return result;
    }
    let found = if instance == 0 {
        // (before an instance, only the three global functions)
        match name {
            b"xrEnumerateInstanceExtensionProperties"
            | b"xrEnumerateApiLayerProperties"
            | b"xrCreateInstance" => function(name),
            _ => return XR_ERROR_HANDLE_INVALID,
        }
    } else {
        let Some(vulkan_enabled) = runtime::runtime().vulkan_enabled(instance) else {
            return XR_ERROR_HANDLE_INVALID;
        };
        function(name).filter(|&(needs_vulkan, _)| !needs_vulkan || vulkan_enabled)
    };
    match found {
        Some((_, f)) => {
            // SAFETY: checked above.
            unsafe { *function_out = f };
            XR_SUCCESS
        }
        None => XR_ERROR_FUNCTION_UNSUPPORTED,
    }
}

/// The loader's negotiation: interface version 1, OpenXR 1.0.
///
/// # Safety
///
/// The loader's structures.
#[no_mangle]
pub unsafe extern "system" fn xrNegotiateLoaderRuntimeInterface(
    loader_info: *const XrNegotiateLoaderInfo,
    runtime_request: *mut XrNegotiateRuntimeRequest,
) -> XrResult {
    if let Some(result) = fail::xr_call("xrNegotiateLoaderRuntimeInterface") {
        return result;
    }
    if loader_info.is_null() || runtime_request.is_null() {
        return XR_ERROR_INITIALIZATION_FAILED;
    }
    // SAFETY: checked above.
    let (info, request) = unsafe { (&*loader_info, &mut *runtime_request) };
    if info.struct_type != XR_LOADER_INTERFACE_STRUCT_LOADER_INFO
        || info.struct_version != XR_LOADER_INFO_STRUCT_VERSION
        || info.struct_size != std::mem::size_of::<XrNegotiateLoaderInfo>()
        || request.struct_type != XR_LOADER_INTERFACE_STRUCT_RUNTIME_REQUEST
        || request.struct_version != XR_RUNTIME_INFO_STRUCT_VERSION
        || request.struct_size != std::mem::size_of::<XrNegotiateRuntimeRequest>()
    {
        return XR_ERROR_INITIALIZATION_FAILED;
    }
    let interface = XR_CURRENT_LOADER_RUNTIME_VERSION;
    if !(info.min_interface_version..=info.max_interface_version).contains(&interface) {
        return XR_ERROR_INITIALIZATION_FAILED;
    }
    // (1.0, patch versions aside, within the loader's range; the version
    // set must be in its range itself)
    let api = xr_make_version(1, 0, 0);
    if !(xr_major_minor(info.min_api_version)..=xr_major_minor(info.max_api_version)).contains(&api)
    {
        return XR_ERROR_INITIALIZATION_FAILED;
    }
    request.runtime_interface_version = interface;
    request.runtime_api_version = api.max(info.min_api_version);
    request.get_instance_proc_addr = Some(get_instance_proc_addr);
    runtime::runtime().negotiated = true;
    XR_SUCCESS
}

/// `xrGetInstanceProcAddr`, for the runtime loaded in the loader's place.
///
/// # Safety
///
/// As OpenXR's.
#[no_mangle]
pub unsafe extern "system" fn xrGetInstanceProcAddr(
    instance: XrInstance,
    name: *const c_char,
    function: *mut Option<PfnXrVoidFunction>,
) -> XrResult {
    // SAFETY: the caller's.
    unsafe { get_instance_proc_addr(instance, name, function) }
}

/// `xrEnumerateApiLayerProperties` (no layers), for the runtime loaded in
/// the loader's place.
///
/// # Safety
///
/// As OpenXR's.
#[no_mangle]
pub unsafe extern "system" fn xrEnumerateApiLayerProperties(
    capacity: u32,
    count: *mut u32,
    properties: *mut XrApiLayerProperties,
) -> XrResult {
    // SAFETY: the caller's.
    unsafe { runtime::enumerate_api_layer_properties(capacity, count, properties) }
}

/// `xrEnumerateInstanceExtensionProperties`, for the runtime loaded in the
/// loader's place.
///
/// # Safety
///
/// As OpenXR's.
#[no_mangle]
pub unsafe extern "system" fn xrEnumerateInstanceExtensionProperties(
    layer_name: *const c_char,
    capacity: u32,
    count: *mut u32,
    properties: *mut XrExtensionProperties,
) -> XrResult {
    // SAFETY: the caller's.
    unsafe {
        runtime::enumerate_instance_extension_properties(layer_name, capacity, count, properties)
    }
}

/// `xrCreateInstance`, for the runtime loaded in the loader's place.
///
/// # Safety
///
/// As OpenXR's.
#[no_mangle]
pub unsafe extern "system" fn xrCreateInstance(
    create_info: *const XrInstanceCreateInfo,
    instance: *mut XrInstance,
) -> XrResult {
    // SAFETY: the caller's.
    unsafe { runtime::create_instance(create_info, instance) }
}

#[cfg(test)]
mod tests;
