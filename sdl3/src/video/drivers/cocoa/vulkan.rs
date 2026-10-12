// Rust translation of src/video/cocoa/SDL_cocoavulkan.m and
// SDL_cocoavulkan.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.
//
// @author Mark Callow, www.edgewise-consulting.com. Based on Jacob Lifshay's
// SDL_x11vulkan.c.

//! Vulkan surfaces for Cocoa windows, through a Vulkan Portability
//! implementation (the Vulkan loader with MoltenVK, or MoltenVK itself):
//! `VK_EXT_metal_surface` (or the older `VK_MVK_macos_surface`) on a Metal
//! view of the window (on the content view's layer for external windows).
//! The loader is loaded at run time, unless the program has
//! `vkGetInstanceProcAddr` already (linked statically).

use std::ffi::{c_char, c_void, CStr};
use std::sync::{Mutex, MutexGuard};

use super::metalview;
use super::window::window_data;
use crate::core::apple::objc::{autoreleasepool, class, is_kind_of, msg_send, release, Id};
use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::WindowID;
use crate::hints;
use crate::loadso::SharedObject;
use crate::video::core::with_window;
use crate::video::vulkan_utils::{
    instance_function, vulkan_create_instance_extensions_list, vulkan_destroy_surface_internal,
    vulkan_get_result_string, PfnVkEnumerateInstanceExtensionProperties, PfnVkGetInstanceProcAddr,
    VkResult, VK_KHR_SURFACE_EXTENSION_NAME, VK_SUCCESS,
};

/// `VK_EXT_METAL_SURFACE_EXTENSION_NAME`.
const VK_EXT_METAL_SURFACE_EXTENSION_NAME: &str = "VK_EXT_metal_surface";
/// `VK_MVK_MACOS_SURFACE_EXTENSION_NAME`.
const VK_MVK_MACOS_SURFACE_EXTENSION_NAME: &str = "VK_MVK_macos_surface";
/// `VK_STRUCTURE_TYPE_METAL_SURFACE_CREATE_INFO_EXT`.
const VK_STRUCTURE_TYPE_METAL_SURFACE_CREATE_INFO_EXT: i32 = 1000217000;
/// `VK_STRUCTURE_TYPE_MACOS_SURFACE_CREATE_INFO_MVK`.
const VK_STRUCTURE_TYPE_MACOS_SURFACE_CREATE_INFO_MVK: i32 = 1000123000;

/// `VkMetalSurfaceCreateInfoEXT`.
#[repr(C)]
struct VkMetalSurfaceCreateInfoEXT {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    p_layer: *const c_void,
}

/// `VkMacOSSurfaceCreateInfoMVK`.
#[repr(C)]
struct VkMacOSSurfaceCreateInfoMVK {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    p_view: *const c_void,
}

/// `PFN_vkCreateMetalSurfaceEXT`.
type PfnVkCreateMetalSurfaceEXT = unsafe extern "system" fn(
    *mut c_void,
    *const VkMetalSurfaceCreateInfoEXT,
    *const c_void,
    *mut u64,
) -> VkResult;
/// `PFN_vkCreateMacOSSurfaceMVK`.
type PfnVkCreateMacOSSurfaceMVK = unsafe extern "system" fn(
    *mut c_void,
    *const VkMacOSSurfaceCreateInfoMVK,
    *const c_void,
    *mut u64,
) -> VkResult;

// <mach-o/dyld.h>, for _NSGetExecutablePath
unsafe extern "C" {
    fn _NSGetExecutablePath(buf: *mut c_char, bufsize: *mut u32) -> i32;
}

/// `defaultPaths`.
const DEFAULT_PATHS: &[&str] = &[
    "@executable_path/../Frameworks/libMoltenVK.dylib",
    "vulkan.framework/vulkan",
    "libvulkan.1.dylib",
    "libvulkan.dylib",
    "MoltenVK.framework/MoltenVK",
    "libMoltenVK.dylib",
    "/usr/local/lib/libvulkan.dylib",
];

/// `vulkan_config.loader_handle`: a loaded library, or `DEFAULT_HANDLE`
/// (`RTLD_DEFAULT`) when the program has `vkGetInstanceProcAddr` itself.
enum LoaderHandle {
    Default,
    Object(#[allow(dead_code)] SharedObject),
}

/// The driver's `_this->vulkan_config`.
struct VulkanConfig {
    loader_handle: LoaderHandle,
    vk_get_instance_proc_addr: PfnVkGetInstanceProcAddr,
}

static VULKAN_CONFIG: Mutex<Option<VulkanConfig>> = Mutex::new(None);

fn config() -> MutexGuard<'static, Option<VulkanConfig>> {
    VULKAN_CONFIG.lock().unwrap_or_else(|e| e.into_inner())
}

/// `dlerror()`, or "" when there is no error.
fn dlerror_text() -> String {
    // SAFETY: dlerror() returns NULL or a NUL-terminated string valid
    // until the next dl call on this thread; it is copied at once.
    unsafe {
        let e = libc::dlerror();
        if e.is_null() {
            String::new()
        } else {
            CStr::from_ptr(e).to_string_lossy().into_owned()
        }
    }
}

/// Translation of `Cocoa_Vulkan_LibraryPathIsUsable()`.
///
/// This function filters the library search paths to only consider absolute paths if they exist.
///
/// dlopen will look for a library by filename. It will only use the provided path if the filename is not found in any
/// DYLD_LIBRARY_PATH.
///
/// With "@executable_path/../Frameworks/libMoltenVK.dylib" being first, this means that it will always circumvent the
/// vulkan loader whenever any libMoltenVK.dylib is found in any discoverable location (which it always will if the
/// vulkan sdk is installed via setup-env.sh for example).
///
/// We don't want to change the priority of the default paths as loading a bundled MoltenVK is genuinely correct. Instead,
/// we expand the requirements for an absolute (and thus @executable_path) path to have to exist in order to be considered
/// for SDL_LoadObject (dlopen).
///
/// This gives us best of both worlds:
/// - if MoltenVK is bundled, it will be used
/// - otherwise it will use the regular vulkan loader path
fn library_path_is_usable(path: &str) -> bool {
    const PREFIX: &str = "@executable_path/";

    let expanded = if path.starts_with('/') {
        path.to_owned()
    } else if let Some(rest) = path.strip_prefix(PREFIX) {
        let mut buf = vec![0u8; libc::PATH_MAX as usize];
        let mut size = buf.len() as u32;
        // SAFETY: a buffer of `size` bytes.
        // if the executable path fails to resolve...
        if unsafe { _NSGetExecutablePath(buf.as_mut_ptr() as *mut c_char, &mut size) } != 0 {
            return true;
        }
        let exe = CStr::from_bytes_until_nul(&buf)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        // ... or is invalid, we still pass it to SDL_LoadObject for the old behavior
        let Some(slash) = exe.rfind('/') else {
            return true;
        };
        format!("{}{}", &exe[..=slash], rest)
    } else {
        // library or framework name, let SDL_LoadObject handle it
        return true;
    };

    std::fs::metadata(expanded).is_ok_and(|m| m.is_file())
}

/// Translation of `Cocoa_Vulkan_LoadLibrary()`.
pub(crate) fn vulkan_load_library(path: Option<&str>) -> Result<()> {
    let mut has_surface_extension = false;
    let mut has_metal_surface_extension = false;
    let mut has_macos_surface_extension = false;

    let mut cfg = config();
    if cfg.is_some() {
        return Err(Error::new("Vulkan Portability library is already loaded."));
    }

    // Load the Vulkan loader library
    let hint = match path {
        Some(_) => None,
        None => hints::get(hints::VULKAN_LIBRARY),
    };
    let path = path.or(hint.as_deref());

    let mut vk_get_instance_proc_addr: Option<PfnVkGetInstanceProcAddr> = None;
    if path.is_none() {
        // Handle the case where Vulkan Portability is linked statically.
        // SAFETY: RTLD_DEFAULT searches every loaded image; the symbol is
        // vkGetInstanceProcAddr, of this type.
        let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"vkGetInstanceProcAddr".as_ptr()) };
        if !p.is_null() {
            // SAFETY: as above.
            vk_get_instance_proc_addr =
                Some(unsafe { std::mem::transmute::<*mut c_void, PfnVkGetInstanceProcAddr>(p) });
        }
    }

    let loader_handle = if vk_get_instance_proc_addr.is_some() {
        LoaderHandle::Default
    } else {
        let paths: Vec<&str> = match path {
            Some(path) => vec![path],
            /* Look for framework or .dylib packaged with the application
             * instead. */
            None => DEFAULT_PATHS.to_vec(),
        };

        let mut found: Option<(SharedObject, &str)> = None;
        for &found_path in &paths {
            if !library_path_is_usable(found_path) {
                continue;
            }
            if let Ok(handle) = SharedObject::load(found_path) {
                found = Some((handle, found_path));
                break;
            }
        }

        let Some((handle, found_path)) = found else {
            return Err(Error::new("Failed to load Vulkan Portability library"));
        };

        // SAFETY: vkGetInstanceProcAddr has this signature.
        match unsafe { handle.function::<PfnVkGetInstanceProcAddr>("vkGetInstanceProcAddr") } {
            Ok(f) => vk_get_instance_proc_addr = Some(f),
            Err(_) => {
                return Err(Error::new(format!(
                    "Failed to find {} in either executable or {}: {}",
                    "vkGetInstanceProcAddr",
                    found_path,
                    dlerror_text()
                )));
            }
        }
        // (the video core keeps vulkan_config.loader_path)
        LoaderHandle::Object(handle)
    };
    let vk_get_instance_proc_addr = vk_get_instance_proc_addr.expect("set above");

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
    // (fail: dropping the handle unloads the library)
    let extensions =
        vulkan_create_instance_extensions_list(vk_enumerate_instance_extension_properties)?;
    for extension in &extensions {
        let name = extension.name();
        if name == VK_KHR_SURFACE_EXTENSION_NAME {
            has_surface_extension = true;
        } else if name == VK_EXT_METAL_SURFACE_EXTENSION_NAME {
            has_metal_surface_extension = true;
        } else if name == VK_MVK_MACOS_SURFACE_EXTENSION_NAME {
            has_macos_surface_extension = true;
        }
    }
    if !has_surface_extension {
        return Err(Error::new(format!(
            "Installed Vulkan Portability library doesn't implement the {VK_KHR_SURFACE_EXTENSION_NAME} extension"
        )));
    } else if !has_metal_surface_extension && !has_macos_surface_extension {
        return Err(Error::new(format!(
            "Installed Vulkan Portability library doesn't implement the {VK_EXT_METAL_SURFACE_EXTENSION_NAME} or {VK_MVK_MACOS_SURFACE_EXTENSION_NAME} extensions"
        )));
    }

    *cfg = Some(VulkanConfig {
        loader_handle,
        vk_get_instance_proc_addr,
    });
    Ok(())
}

/// Translation of `Cocoa_Vulkan_UnloadLibrary()` (the `RTLD_DEFAULT`
/// handle isn't unloaded).
pub(crate) fn vulkan_unload_library() {
    if let Some(cfg) = config().take() {
        if let LoaderHandle::Object(handle) = cfg.loader_handle {
            drop(handle);
        }
    }
}

/// The loader's `vkGetInstanceProcAddr` (`vulkan_config.vkGetInstanceProcAddr`).
pub(crate) fn vk_get_instance_proc_addr() -> Option<usize> {
    config()
        .as_ref()
        .map(|c| c.vk_get_instance_proc_addr as usize)
}

/// Translation of `Cocoa_Vulkan_GetInstanceExtensions()`.
pub(crate) fn vulkan_get_instance_extensions() -> Vec<&'static str> {
    // (extensionsForCocoa)
    vec![
        VK_KHR_SURFACE_EXTENSION_NAME,
        VK_EXT_METAL_SURFACE_EXTENSION_NAME,
    ]
}

/// Create a surface on `layer` (`VK_EXT_metal_surface`) or `view`
/// (`VK_MVK_macos_surface`).
///
/// # Safety
///
/// The functions belong to `instance`; the layer or view is valid.
unsafe fn create_surface_with(
    instance: *mut c_void,
    allocator: *const c_void,
    vk_create_metal_surface_ext: Option<PfnVkCreateMetalSurfaceEXT>,
    vk_create_macos_surface_mvk: Option<PfnVkCreateMacOSSurfaceMVK>,
    layer: *const c_void,
    view: *const c_void,
    mvk_allocator: *const c_void,
) -> Result<u64> {
    let mut surface: u64 = 0;
    if let Some(vk_create_metal_surface_ext) = vk_create_metal_surface_ext {
        let create_info = VkMetalSurfaceCreateInfoEXT {
            s_type: VK_STRUCTURE_TYPE_METAL_SURFACE_CREATE_INFO_EXT,
            p_next: std::ptr::null(),
            flags: 0,
            p_layer: layer,
        };
        // SAFETY: the caller's instance and layer.
        let rc =
            unsafe { vk_create_metal_surface_ext(instance, &create_info, allocator, &mut surface) };
        if rc != VK_SUCCESS {
            return Err(Error::new(format!(
                "vkCreateMetalSurfaceEXT failed: {}",
                vulkan_get_result_string(rc)
            )));
        }
    } else if let Some(vk_create_macos_surface_mvk) = vk_create_macos_surface_mvk {
        let create_info = VkMacOSSurfaceCreateInfoMVK {
            s_type: VK_STRUCTURE_TYPE_MACOS_SURFACE_CREATE_INFO_MVK,
            p_next: std::ptr::null(),
            flags: 0,
            p_view: view,
        };
        // SAFETY: the caller's instance and view.
        let rc = unsafe {
            vk_create_macos_surface_mvk(instance, &create_info, mvk_allocator, &mut surface)
        };
        if rc != VK_SUCCESS {
            return Err(Error::new(format!(
                "vkCreateMacOSSurfaceMVK failed: {}",
                vulkan_get_result_string(rc)
            )));
        }
    }
    Ok(surface)
}

/// Translation of `Cocoa_Vulkan_CreateSurfaceViaMetalView()`.
fn create_surface_via_metal_view(
    window: WindowID,
    instance: *mut c_void,
    allocator: *const c_void,
    vk_create_metal_surface_ext: Option<PfnVkCreateMetalSurfaceEXT>,
    vk_create_macos_surface_mvk: Option<PfnVkCreateMacOSSurfaceMVK>,
) -> Result<u64> {
    let Some(metalview) = metalview::metal_create_view(window) else {
        return Err(Error::new("Couldn't create Metal view"));
    };

    let layer = metalview::metal_get_layer(metalview).unwrap_or(0);
    // SAFETY: the instance's functions, the view's layer and the view.
    // (upstream passes NULL, not the allocator, to vkCreateMacOSSurfaceMVK
    // here)
    let result = unsafe {
        create_surface_with(
            instance,
            allocator,
            vk_create_metal_surface_ext,
            vk_create_macos_surface_mvk,
            layer as *const c_void,
            metalview as *const c_void,
            std::ptr::null(),
        )
    };
    if result.is_err() {
        metalview::metal_destroy_view(metalview);
        return result;
    }

    /* Unfortunately there's no SDL_Vulkan_DestroySurface function we can call
     * Metal_DestroyView from. Right now the metal view's ref count is +2 (one
     * from returning a new view object in CreateView, and one because it's
     * a subview of the window.) If we release the view here to make it +1, it
     * will be destroyed when the window is destroyed.
     *
     * TODO: Now that we have SDL_Vulkan_DestroySurface someone with enough
     * knowledge of Metal can proceed. */
    // SAFETY: the reference metal_create_view() returned (CFBridgingRelease).
    unsafe { release(metalview as Id) };

    result // success!
}

/// Translation of `Cocoa_Vulkan_CreateSurface()`.
pub(crate) fn vulkan_create_surface(
    window: WindowID,
    instance: usize,
    allocator: usize,
) -> Result<u64> {
    let Some(get) = config().as_ref().map(|c| c.vk_get_instance_proc_addr) else {
        return Err(Error::new("Vulkan is not loaded"));
    };
    let instance = instance as *mut c_void;
    let allocator = allocator as *const c_void;
    // SAFETY: the loader's vkGetInstanceProcAddr and the caller's instance;
    // the functions have these types.
    let (vk_create_metal_surface_ext, vk_create_macos_surface_mvk) = unsafe {
        (
            instance_function::<PfnVkCreateMetalSurfaceEXT>(
                get,
                instance,
                c"vkCreateMetalSurfaceEXT",
            ),
            instance_function::<PfnVkCreateMacOSSurfaceMVK>(
                get,
                instance,
                c"vkCreateMacOSSurfaceMVK",
            ),
        )
    };

    if vk_create_metal_surface_ext.is_none() && vk_create_macos_surface_mvk.is_none() {
        return Err(Error::new(format!(
            "{VK_EXT_METAL_SURFACE_EXTENSION_NAME} or {VK_MVK_MACOS_SURFACE_EXTENSION_NAME} extensions are not enabled in the Vulkan instance."
        )));
    }

    let external = with_window(window, |w| w.flags().contains(WindowFlags::EXTERNAL))?;
    if external {
        autoreleasepool(|| {
            let data = window_data(window).ok_or_else(|| Error::new("Invalid window"))?;
            let view = data.sdl_content_view.get();
            // SAFETY: the window's content view and CAMetalLayer's factory method.
            unsafe {
                let layer: Id = msg_send![view, layer];
                if !is_kind_of(layer, class!(CAMetalLayer)) {
                    let metal_layer: Id = msg_send![class!(CAMetalLayer), layer];
                    let _: () = msg_send![view, setLayer: metal_layer];
                }
                let layer: Id = msg_send![view, layer];

                create_surface_with(
                    instance,
                    allocator,
                    vk_create_metal_surface_ext,
                    vk_create_macos_surface_mvk,
                    layer as *const c_void,
                    view as *const c_void,
                    allocator,
                )
            }
        })
    } else {
        create_surface_via_metal_view(
            window,
            instance,
            allocator,
            vk_create_metal_surface_ext,
            vk_create_macos_surface_mvk,
        )
    }
}

/// Translation of `Cocoa_Vulkan_DestroySurface()`.
pub(crate) fn vulkan_destroy_surface(instance: usize, surface: u64, allocator: usize) {
    let get = config().as_ref().map(|c| c.vk_get_instance_proc_addr);
    if let Some(get) = get {
        // SAFETY: the loader's vkGetInstanceProcAddr and the caller's
        // instance, surface and allocator.
        unsafe {
            vulkan_destroy_surface_internal(
                get,
                instance as *mut c_void,
                surface,
                allocator as *const c_void,
            )
        };
        // TODO: Add CFBridgingRelease(metalview) here perhaps?
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn library_paths() {
        // Names go to dlopen as they are; absolute paths only if they exist.
        assert!(super::library_path_is_usable("libvulkan.1.dylib"));
        assert!(super::library_path_is_usable("vulkan.framework/vulkan"));
        assert!(!super::library_path_is_usable("/no/such/libvulkan.dylib"));
        // Relative to the test executable, which has no ../Frameworks.
        assert!(!super::library_path_is_usable(
            "@executable_path/../Frameworks/libMoltenVK.dylib"
        ));
        let exe = std::env::current_exe().unwrap();
        assert!(super::library_path_is_usable(exe.to_str().unwrap()));
        let name = exe.file_name().unwrap().to_str().unwrap();
        assert!(super::library_path_is_usable(&format!(
            "@executable_path/{name}"
        )));
    }
}
