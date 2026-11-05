// The test runtime's own tests: what can be checked without a Vulkan
// driver (the sdl3 crate's gpu_openxr test runs the rest on lavapipe).
// Part of the repository's own test code (not a translation); see LICENSE.txt.

use std::ffi::{c_char, CString};
use std::ptr::{null, null_mut};

use super::*;
use crate::runtime::*;

fn gipa(instance: XrInstance, name: &str) -> (XrResult, bool) {
    let name = CString::new(name).unwrap();
    let mut f = None;
    // SAFETY: a name and an out-pointer.
    let result = unsafe { get_instance_proc_addr(instance, name.as_ptr(), &mut f) };
    (result, f.is_some())
}

fn name_array(name: &str) -> [c_char; 128] {
    let mut a = [0 as c_char; 128];
    for (d, &b) in a.iter_mut().zip(name.as_bytes()) {
        *d = b as c_char;
    }
    a
}

/// An instance with these extensions and API version.
fn instance(extensions: &[&str], api_version: XrVersion) -> Result<XrInstance, XrResult> {
    let names: Vec<CString> = extensions
        .iter()
        .map(|e| CString::new(*e).unwrap())
        .collect();
    #[cfg_attr(not(target_os = "android"), allow(unused_mut))]
    let mut pointers: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
    // (on Android, the app's information, which the runtime needs: a
    // program the shell runs has no Java VM to check it against)
    #[cfg(target_os = "android")]
    let android = android_info(&mut pointers);
    #[cfg(target_os = "android")]
    let next: *const std::ffi::c_void = (&android as *const XrInstanceCreateInfoAndroidKHR).cast();
    #[cfg(not(target_os = "android"))]
    let next = null();
    let info = XrInstanceCreateInfo {
        ty: XR_TYPE_INSTANCE_CREATE_INFO,
        next,
        create_flags: 0,
        application_info: XrApplicationInfo {
            application_name: name_array("runtime tests"),
            application_version: 1,
            engine_name: name_array(""),
            engine_version: 0,
            api_version,
        },
        enabled_api_layer_count: 0,
        enabled_api_layer_names: null(),
        enabled_extension_count: pointers.len() as u32,
        enabled_extension_names: pointers.as_ptr(),
    };
    let mut instance = 0;
    // SAFETY: the info and its arrays, alive for the call.
    match unsafe { create_instance(&info, &mut instance) } {
        XR_SUCCESS => Ok(instance),
        e => Err(e),
    }
}

/// The Android structure of an instance's creation, with stand-ins for the
/// VM and the activity, its extension added to `extensions`.
#[cfg(target_os = "android")]
fn android_info(extensions: &mut Vec<*const c_char>) -> XrInstanceCreateInfoAndroidKHR {
    extensions.push(c"XR_KHR_android_create_instance".as_ptr());
    XrInstanceCreateInfoAndroidKHR {
        ty: XR_TYPE_INSTANCE_CREATE_INFO_ANDROID_KHR,
        next: null(),
        application_vm: std::ptr::NonNull::<u8>::dangling().as_ptr().cast(),
        application_activity: std::ptr::NonNull::<u8>::dangling().as_ptr().cast(),
    }
}

const VULKAN: &[&str] = &[XR_KHR_VULKAN_ENABLE2_EXTENSION_NAME];
const API_1_0: XrVersion = xr_make_version(1, 0, 63);

#[test]
fn proc_addr_follows_the_specification() {
    // (before an instance)
    assert_eq!(gipa(0, "xrCreateInstance"), (XR_SUCCESS, true));
    assert_eq!(gipa(0, "xrEnumerateApiLayerProperties"), (XR_SUCCESS, true));
    assert_eq!(
        gipa(0, "xrEnumerateInstanceExtensionProperties"),
        (XR_SUCCESS, true)
    );
    assert_eq!(gipa(0, "xrGetSystem"), (XR_ERROR_HANDLE_INVALID, false));
    // (XR_KHR_loader_init, on Android only)
    assert_eq!(
        gipa(0, "xrInitializeLoaderKHR"),
        if cfg!(target_os = "android") {
            (XR_SUCCESS, true)
        } else {
            (XR_ERROR_HANDLE_INVALID, false)
        }
    );
    assert_eq!(
        gipa(0xdead_0000, "xrGetSystem"),
        (XR_ERROR_HANDLE_INVALID, false)
    );
    // (the extension's functions with the extension only)
    let plain = instance(&[], API_1_0).unwrap();
    let with = instance(VULKAN, API_1_0).unwrap();
    assert_eq!(gipa(plain, "xrGetSystem"), (XR_SUCCESS, true));
    assert_eq!(
        gipa(plain, "xrCreateVulkanInstanceKHR"),
        (XR_ERROR_FUNCTION_UNSUPPORTED, false)
    );
    assert_eq!(gipa(with, "xrCreateVulkanInstanceKHR"), (XR_SUCCESS, true));
    assert_eq!(
        gipa(with, "xrGetInstanceProperties"),
        (XR_ERROR_FUNCTION_UNSUPPORTED, false)
    );
    let mut f = None;
    // SAFETY: a null name is refused.
    assert_eq!(
        unsafe { get_instance_proc_addr(with, null(), &mut f) },
        XR_ERROR_VALIDATION_FAILURE
    );
    // SAFETY: the instances made above.
    unsafe {
        assert_eq!(destroy_instance(plain), XR_SUCCESS);
        assert_eq!(destroy_instance(with), XR_SUCCESS);
        assert_eq!(destroy_instance(with), XR_ERROR_HANDLE_INVALID);
    }
    assert_eq!(gipa(with, "xrGetSystem"), (XR_ERROR_HANDLE_INVALID, false));
}

#[test]
fn negotiation_checks_the_loader_structures() {
    let info = |min_api, max_api| XrNegotiateLoaderInfo {
        struct_type: XR_LOADER_INTERFACE_STRUCT_LOADER_INFO,
        struct_version: XR_LOADER_INFO_STRUCT_VERSION,
        struct_size: std::mem::size_of::<XrNegotiateLoaderInfo>(),
        min_interface_version: 1,
        max_interface_version: 1,
        min_api_version: min_api,
        max_api_version: max_api,
    };
    let request = || XrNegotiateRuntimeRequest {
        struct_type: XR_LOADER_INTERFACE_STRUCT_RUNTIME_REQUEST,
        struct_version: XR_RUNTIME_INFO_STRUCT_VERSION,
        struct_size: std::mem::size_of::<XrNegotiateRuntimeRequest>(),
        runtime_interface_version: 0,
        runtime_api_version: 0,
        get_instance_proc_addr: None,
    };
    let negotiate = |info: &XrNegotiateLoaderInfo, request: &mut XrNegotiateRuntimeRequest| {
        // SAFETY: the structures.
        unsafe { xrNegotiateLoaderRuntimeInterface(info, request) }
    };
    // (a 1.0 loader, a 1.1 loader)
    for (min, max) in [
        (xr_make_version(1, 0, 0), xr_make_version(1, 0, 20)),
        (xr_make_version(1, 0, 0), xr_make_version(1, 1, 65)),
    ] {
        let mut r = request();
        assert_eq!(negotiate(&info(min, max), &mut r), XR_SUCCESS);
        assert_eq!(r.runtime_interface_version, 1);
        assert_eq!(r.runtime_api_version, xr_make_version(1, 0, 0));
        assert!(r.get_instance_proc_addr.is_some());
    }
    // (a loader of 1.1 only)
    let mut r = request();
    let only_1_1 = info(xr_make_version(1, 1, 0), xr_make_version(1, 1, 65));
    assert_eq!(negotiate(&only_1_1, &mut r), XR_ERROR_INITIALIZATION_FAILED);
    let mut bad = info(xr_make_version(1, 0, 0), xr_make_version(1, 0, 20));
    bad.struct_size -= 8;
    assert_eq!(
        negotiate(&bad, &mut request()),
        XR_ERROR_INITIALIZATION_FAILED
    );
    let mut bad = info(xr_make_version(1, 0, 0), xr_make_version(1, 0, 20));
    bad.min_interface_version = 2;
    bad.max_interface_version = 2;
    assert_eq!(
        negotiate(&bad, &mut request()),
        XR_ERROR_INITIALIZATION_FAILED
    );
    let mut r = request();
    r.struct_type = XR_LOADER_INTERFACE_STRUCT_LOADER_INFO;
    let good = info(xr_make_version(1, 0, 0), xr_make_version(1, 0, 20));
    assert_eq!(negotiate(&good, &mut r), XR_ERROR_INITIALIZATION_FAILED);
}

#[test]
fn extensions_two_call_idiom() {
    // (XR_KHR_D3D12_enable on Windows only, XR_KHR_android_create_instance
    // on Android only)
    let expected: &[(&str, u32)] = if cfg!(target_os = "android") {
        &[
            (
                XR_KHR_VULKAN_ENABLE2_EXTENSION_NAME,
                XR_KHR_VULKAN_ENABLE2_SPEC_VERSION,
            ),
            (
                XR_KHR_ANDROID_CREATE_INSTANCE_EXTENSION_NAME,
                XR_KHR_ANDROID_CREATE_INSTANCE_SPEC_VERSION,
            ),
        ]
    } else if cfg!(windows) {
        &[
            (
                XR_KHR_VULKAN_ENABLE2_EXTENSION_NAME,
                XR_KHR_VULKAN_ENABLE2_SPEC_VERSION,
            ),
            (
                XR_KHR_D3D12_ENABLE_EXTENSION_NAME,
                XR_KHR_D3D12_ENABLE_SPEC_VERSION,
            ),
        ]
    } else {
        &[(
            XR_KHR_VULKAN_ENABLE2_EXTENSION_NAME,
            XR_KHR_VULKAN_ENABLE2_SPEC_VERSION,
        )]
    };
    let n = expected.len() as u32;
    let mut count = 0;
    // SAFETY: the count query, then arrays of that many.
    unsafe {
        assert_eq!(
            xrEnumerateInstanceExtensionProperties(null(), 0, &mut count, null_mut()),
            XR_SUCCESS
        );
        assert_eq!(count, n);
        let mut p: Vec<XrExtensionProperties> = (0..n).map(|_| std::mem::zeroed()).collect();
        assert_eq!(
            xrEnumerateInstanceExtensionProperties(null(), n, &mut count, p.as_mut_ptr()),
            XR_ERROR_VALIDATION_FAILURE,
            "untyped"
        );
        for p in &mut p {
            p.ty = XR_TYPE_EXTENSION_PROPERTIES;
        }
        assert_eq!(
            xrEnumerateInstanceExtensionProperties(null(), n - 1, &mut count, p.as_mut_ptr()),
            if n > 1 {
                XR_ERROR_SIZE_INSUFFICIENT
            } else {
                XR_SUCCESS
            }
        );
        assert_eq!(
            xrEnumerateInstanceExtensionProperties(null(), n, &mut count, p.as_mut_ptr()),
            XR_SUCCESS
        );
        for (p, &(name, version)) in p.iter().zip(expected) {
            let found = CStr::from_ptr(p.extension_name.as_ptr());
            assert_eq!(found.to_str().unwrap(), name);
            assert_eq!(p.extension_version, version);
        }
        let layer = c"XR_APILAYER_LUNARG_core_validation";
        assert_eq!(
            xrEnumerateInstanceExtensionProperties(layer.as_ptr(), 0, &mut count, null_mut()),
            XR_ERROR_API_LAYER_NOT_PRESENT
        );
        assert_eq!(
            xrEnumerateApiLayerProperties(0, &mut count, null_mut()),
            XR_SUCCESS
        );
        assert_eq!(count, 0);
    }
}

#[test]
fn instance_creation_checks_its_info() {
    assert_eq!(
        instance(&["XR_SDL3_test_unknown"], API_1_0),
        Err(XR_ERROR_EXTENSION_NOT_PRESENT)
    );
    assert_eq!(
        instance(VULKAN, xr_make_version(2, 0, 0)),
        Err(XR_ERROR_API_VERSION_UNSUPPORTED)
    );
    assert_eq!(
        instance(VULKAN, xr_make_version(1, 1, 0)),
        Err(XR_ERROR_API_VERSION_UNSUPPORTED)
    );
    let mut info: XrInstanceCreateInfo = unsafe { std::mem::zeroed() };
    info.ty = XR_TYPE_INSTANCE_CREATE_INFO;
    info.application_info.api_version = API_1_0;
    let mut handle = 0;
    // SAFETY: an info without a name, then without a type.
    unsafe {
        assert_eq!(create_instance(&info, &mut handle), XR_ERROR_NAME_INVALID);
        info.ty = 0;
        assert_eq!(
            create_instance(&info, &mut handle),
            XR_ERROR_VALIDATION_FAILURE
        );
    }
}

#[test]
fn system_requirements_and_session_order() {
    let instance = instance(VULKAN, API_1_0).unwrap();
    let mut system_id = 0;
    let get = |form_factor| XrSystemGetInfo {
        ty: XR_TYPE_SYSTEM_GET_INFO,
        next: null(),
        form_factor,
    };
    let mut requirements = XrGraphicsRequirementsVulkan2KHR {
        ty: XR_TYPE_GRAPHICS_REQUIREMENTS_VULKAN2_KHR,
        next: null_mut(),
        min_api_version_supported: 0,
        max_api_version_supported: 0,
    };
    let session_info = XrSessionCreateInfo {
        ty: XR_TYPE_SESSION_CREATE_INFO,
        next: null(),
        create_flags: 0,
        system_id: SYSTEM_ID,
    };
    let mut session = 0;
    let mut event: XrEventDataBuffer = unsafe { std::mem::zeroed() };
    event.ty = XR_TYPE_EVENT_DATA_BUFFER;
    // SAFETY: the instance, infos and out-pointers.
    unsafe {
        // (no system yet)
        assert_eq!(
            get_vulkan_graphics_requirements2(instance, SYSTEM_ID, &mut requirements),
            XR_ERROR_SYSTEM_INVALID
        );
        assert_eq!(
            get_system(
                instance,
                &get(XR_FORM_FACTOR_HANDHELD_DISPLAY),
                &mut system_id
            ),
            XR_ERROR_FORM_FACTOR_UNSUPPORTED
        );
        assert_eq!(system_id, 0);
        assert_eq!(
            get_system(
                instance,
                &get(XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY),
                &mut system_id
            ),
            XR_SUCCESS
        );
        assert_eq!(system_id, SYSTEM_ID);
        assert_eq!(
            create_session(instance, &session_info, &mut session),
            XR_ERROR_GRAPHICS_REQUIREMENTS_CALL_MISSING
        );
        assert_eq!(
            get_vulkan_graphics_requirements2(instance, SYSTEM_ID, &mut requirements),
            XR_SUCCESS
        );
        assert_eq!(
            requirements.min_api_version_supported,
            xr_make_version(1, 0, 0)
        );
        assert_eq!(
            requirements.max_api_version_supported,
            xr_make_version(1, 3, 0)
        );
        // (no graphics binding)
        assert_eq!(
            create_session(instance, &session_info, &mut session),
            XR_ERROR_GRAPHICS_DEVICE_INVALID
        );
        assert_eq!(session, 0);
        assert_eq!(poll_event(instance, &mut event), XR_EVENT_UNAVAILABLE);
        assert_eq!(destroy_instance(instance), XR_SUCCESS);
    }
}

/// `XR_KHR_D3D12_enable`'s function with the extension only, its
/// requirements (the adapter DXGI gives, feature level 11_0), and the
/// failure point that gives a LUID no adapter has.
#[cfg(windows)]
#[test]
fn d3d12_requirements() {
    const D3D12: &[&str] = &[XR_KHR_D3D12_ENABLE_EXTENSION_NAME];
    let plain = instance(VULKAN, API_1_0).unwrap();
    let with = instance(D3D12, API_1_0).unwrap();
    assert_eq!(
        gipa(plain, "xrGetD3D12GraphicsRequirementsKHR"),
        (XR_ERROR_FUNCTION_UNSUPPORTED, false)
    );
    assert_eq!(
        gipa(with, "xrGetD3D12GraphicsRequirementsKHR"),
        (XR_SUCCESS, true)
    );
    assert_eq!(
        gipa(with, "xrCreateVulkanInstanceKHR"),
        (XR_ERROR_FUNCTION_UNSUPPORTED, false)
    );
    let mut requirements = XrGraphicsRequirementsD3D12KHR {
        ty: XR_TYPE_GRAPHICS_REQUIREMENTS_D3D12_KHR,
        next: null_mut(),
        adapter_luid: Luid::default(),
        min_feature_level: 0,
    };
    let mut system_id = 0;
    let get = XrSystemGetInfo {
        ty: XR_TYPE_SYSTEM_GET_INFO,
        next: null(),
        form_factor: XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY,
    };
    // SAFETY: the instances, infos and out-pointers.
    unsafe {
        assert_eq!(
            get_d3d12_graphics_requirements(with, SYSTEM_ID, &mut requirements),
            XR_ERROR_SYSTEM_INVALID
        );
        assert_eq!(get_system(with, &get, &mut system_id), XR_SUCCESS);
        assert_eq!(
            get_d3d12_graphics_requirements(with, SYSTEM_ID, &mut requirements),
            XR_SUCCESS
        );
        assert_eq!(requirements.min_feature_level, 0xb000);
        assert_ne!(requirements.adapter_luid, Luid::default());
        assert_ne!(requirements.adapter_luid, NO_ADAPTER_LUID);
        assert_eq!(destroy_instance(with), XR_SUCCESS);
        assert_eq!(destroy_instance(plain), XR_SUCCESS);
    }
}

#[cfg(target_os = "android")]
#[test]
fn android_loader_initialization_and_instance_information() {
    use std::ffi::c_void;
    let init = |info: *const c_void| {
        // SAFETY: the runtime's function, with a structure (or null).
        unsafe { crate::android::initialize_loader(info.cast()) }
    };
    let dangling: *mut c_void = std::ptr::NonNull::<u8>::dangling().as_ptr().cast();
    let android = |vm, context| XrLoaderInitInfoAndroidKHR {
        ty: XR_TYPE_LOADER_INIT_INFO_ANDROID_KHR,
        next: null(),
        application_vm: vm,
        application_context: context,
    };
    // (no structure, another structure, null pointers: a program the shell
    // runs has no Java VM to check them against, beyond that)
    assert_eq!(init(null()), XR_ERROR_VALIDATION_FAILURE);
    let other = XrLoaderInitInfoBaseHeaderKHR {
        ty: XR_TYPE_INSTANCE_CREATE_INFO,
        next: null(),
    };
    assert_eq!(
        init((&other as *const XrLoaderInitInfoBaseHeaderKHR).cast()),
        XR_ERROR_VALIDATION_FAILURE
    );
    for (vm, context) in [
        (std::ptr::null_mut(), dangling),
        (dangling, std::ptr::null_mut()),
    ] {
        let info = android(vm, context);
        assert_eq!(
            init((&info as *const XrLoaderInitInfoAndroidKHR).cast()),
            XR_ERROR_VALIDATION_FAILURE
        );
    }

    // (the instance: the extension and its structure, both needed)
    let names = [c"XR_KHR_vulkan_enable2".as_ptr()];
    let mut info = XrInstanceCreateInfo {
        ty: XR_TYPE_INSTANCE_CREATE_INFO,
        next: null(),
        create_flags: 0,
        application_info: XrApplicationInfo {
            application_name: name_array("runtime tests"),
            application_version: 1,
            engine_name: name_array(""),
            engine_version: 0,
            api_version: API_1_0,
        },
        enabled_api_layer_count: 0,
        enabled_api_layer_names: null(),
        enabled_extension_count: 1,
        enabled_extension_names: names.as_ptr(),
    };
    let mut handle = 0;
    // SAFETY: the info and its arrays.
    unsafe {
        assert_eq!(
            create_instance(&info, &mut handle),
            XR_ERROR_VALIDATION_FAILURE,
            "no Android structure"
        );
        let mut with = Vec::from(names);
        let structure = android_info(&mut with);
        info.enabled_extension_names = with.as_ptr();
        info.enabled_extension_count = 2;
        assert_eq!(
            create_instance(&info, &mut handle),
            XR_ERROR_VALIDATION_FAILURE,
            "no Android structure, the extension enabled"
        );
        info.next = (&structure as *const XrInstanceCreateInfoAndroidKHR).cast();
        info.enabled_extension_count = 1;
        assert_eq!(
            create_instance(&info, &mut handle),
            XR_ERROR_VALIDATION_FAILURE,
            "the extension not enabled"
        );
        let mut no_vm = android_info(&mut Vec::new());
        no_vm.application_vm = std::ptr::null_mut();
        info.next = (&no_vm as *const XrInstanceCreateInfoAndroidKHR).cast();
        info.enabled_extension_count = 2;
        assert_eq!(
            create_instance(&info, &mut handle),
            XR_ERROR_VALIDATION_FAILURE,
            "no VM"
        );
        info.next = (&structure as *const XrInstanceCreateInfoAndroidKHR).cast();
        assert_eq!(create_instance(&info, &mut handle), XR_SUCCESS);
        assert_eq!(destroy_instance(handle), XR_SUCCESS);
    }
}
