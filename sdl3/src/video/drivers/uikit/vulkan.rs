// Rust translation of src/video/uikit/SDL_uikitvulkan.m and
// SDL_uikitvulkan.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.
//
// @author Mark Callow, www.edgewise-consulting.com. Based on Jacob Lifshay's
// SDL_x11vulkan.c.

//! Vulkan surfaces for UIKit windows, through a Vulkan Portability
//! implementation (MoltenVK, which an app bundles, or the Vulkan loader):
//! `VK_EXT_metal_surface` (or the older `VK_MVK_ios_surface`) on a Metal
//! view of the window ([`super::metalview`]). The library is the one
//! `SDL_HINT_VULKAN_LIBRARY` names, else the program's own
//! `vkGetInstanceProcAddr` (Vulkan Portability linked statically), else
//! `libvulkan.dylib`, loaded at run time.
//!
//! Note (upstream): SDL_vulkan.h says SDL attempts to load
//! `libMoltenVK.dylib` on iOS; upstream's UIKit driver tries
//! `libvulkan.dylib` only, as here. An app that bundles MoltenVK's
//! framework (`Frameworks/MoltenVK.framework`) names it in the hint, as
//! `@executable_path/Frameworks/MoltenVK.framework/MoltenVK`.

use std::ffi::{c_void, CStr};
use std::sync::{Mutex, MutexGuard};

use super::metalview;
use crate::core::apple::objc::{release, Id};
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::hints;
use crate::loadso::SharedObject;
use crate::video::vulkan_utils::{
    instance_function, vulkan_create_instance_extensions_list, vulkan_destroy_surface_internal,
    vulkan_get_result_string, PfnVkEnumerateInstanceExtensionProperties, PfnVkGetInstanceProcAddr,
    VkResult, VK_KHR_SURFACE_EXTENSION_NAME, VK_SUCCESS,
};

/// `VK_EXT_METAL_SURFACE_EXTENSION_NAME`.
const VK_EXT_METAL_SURFACE_EXTENSION_NAME: &str = "VK_EXT_metal_surface";
/// `VK_MVK_IOS_SURFACE_EXTENSION_NAME`.
const VK_MVK_IOS_SURFACE_EXTENSION_NAME: &str = "VK_MVK_ios_surface";
/// `VK_STRUCTURE_TYPE_METAL_SURFACE_CREATE_INFO_EXT`.
const VK_STRUCTURE_TYPE_METAL_SURFACE_CREATE_INFO_EXT: i32 = 1000217000;
/// `VK_STRUCTURE_TYPE_IOS_SURFACE_CREATE_INFO_MVK`.
const VK_STRUCTURE_TYPE_IOS_SURFACE_CREATE_INFO_MVK: i32 = 1000122000;

/// `VkMetalSurfaceCreateInfoEXT`.
#[repr(C)]
struct VkMetalSurfaceCreateInfoEXT {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    p_layer: *const c_void,
}

/// `VkIOSSurfaceCreateInfoMVK`.
#[repr(C)]
struct VkIOSSurfaceCreateInfoMVK {
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
/// `PFN_vkCreateIOSSurfaceMVK`.
type PfnVkCreateIOSSurfaceMVK = unsafe extern "system" fn(
    *mut c_void,
    *const VkIOSSurfaceCreateInfoMVK,
    *const c_void,
    *mut u64,
) -> VkResult;

/// `defaultPaths`.
const DEFAULT_PATHS: &[&str] = &["libvulkan.dylib"];

/// `vulkan_config.loader_handle`: a loaded library, or `DEFAULT_HANDLE`
/// (`RTLD_DEFAULT`: since libSDL is static, could use `RTLD_SELF`; using
/// `RTLD_DEFAULT` is future proofing) when the program has
/// `vkGetInstanceProcAddr` itself.
enum LoaderHandle {
    Default,
    Object(#[allow(dead_code)] SharedObject),
}

/// The driver's `_this->vulkan_config`.
struct VulkanConfig {
    loader_handle: LoaderHandle,
    /// `vulkan_config.loader_path`: the path the library was loaded from
    /// (`None` for the program's own `vkGetInstanceProcAddr`, where
    /// upstream leaves it as it was, and for a default name).
    loader_path: Option<String>,
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

/// Translation of `UIKit_Vulkan_LoadLibrary()`.
pub(crate) fn vulkan_load_library(path: Option<&str>) -> Result<()> {
    let mut has_surface_extension = false;
    let mut has_metal_surface_extension = false;
    let mut has_ios_surface_extension = false;

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
        let paths: &[&str] = match &path {
            Some(path) => std::slice::from_ref(path),
            // Look for the .dylib packaged with the application instead.
            None => DEFAULT_PATHS,
        };

        let Some(handle) = paths
            .iter()
            .find_map(|found_path| SharedObject::load(found_path).ok())
        else {
            return Err(Error::new("Failed to load Vulkan Portability library"));
        };

        // SAFETY: vkGetInstanceProcAddr has this signature.
        match unsafe { handle.function::<PfnVkGetInstanceProcAddr>("vkGetInstanceProcAddr") } {
            Ok(f) => vk_get_instance_proc_addr = Some(f),
            Err(_) => {
                // (fail: dropping the handle unloads the library, which
                // upstream leaves loaded)
                return Err(Error::new(format!(
                    "Failed to find {} in either executable or {}: {}",
                    "vkGetInstanceProcAddr",
                    "linked Vulkan Portability library",
                    dlerror_text()
                )));
            }
        }
        LoaderHandle::Object(handle)
    };
    // FIXME (upstream): `path` is copied to vulkan_config.loader_path once
    // a library is loaded, which is NULL when it's a default name
    // (SDL_strlcpy() of NULL); none is kept then.
    let loader_path = match loader_handle {
        LoaderHandle::Object(_) => path.map(str::to_owned),
        LoaderHandle::Default => None,
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
        return Err(Error::new(
            "No vkEnumerateInstanceExtensionProperties found.",
        ));
    };

    let extensions =
        vulkan_create_instance_extensions_list(vk_enumerate_instance_extension_properties)?;
    for extension in &extensions {
        let name = extension.name();
        if name == VK_KHR_SURFACE_EXTENSION_NAME {
            has_surface_extension = true;
        } else if name == VK_EXT_METAL_SURFACE_EXTENSION_NAME {
            has_metal_surface_extension = true;
        } else if name == VK_MVK_IOS_SURFACE_EXTENSION_NAME {
            has_ios_surface_extension = true;
        }
    }

    if !has_surface_extension {
        return Err(Error::new(format!(
            "Installed Vulkan Portability doesn't implement the {VK_KHR_SURFACE_EXTENSION_NAME} extension"
        )));
    } else if !has_metal_surface_extension && !has_ios_surface_extension {
        return Err(Error::new(format!(
            "Installed Vulkan Portability doesn't implement the {VK_EXT_METAL_SURFACE_EXTENSION_NAME} or {VK_MVK_IOS_SURFACE_EXTENSION_NAME} extensions"
        )));
    }

    *cfg = Some(VulkanConfig {
        loader_handle,
        loader_path,
        vk_get_instance_proc_addr,
    });
    Ok(())
}

/// The path the library was loaded from (`vulkan_config.loader_path`).
pub(crate) fn vulkan_loader_path() -> Option<String> {
    config().as_ref().and_then(|c| c.loader_path.clone())
}

/// Translation of `UIKit_Vulkan_UnloadLibrary()` (the `DEFAULT_HANDLE`
/// isn't unloaded).
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

/// Translation of `UIKit_Vulkan_GetInstanceExtensions()`.
pub(crate) fn vulkan_get_instance_extensions() -> Vec<&'static str> {
    // (extensionsForUIKit)
    vec![
        VK_KHR_SURFACE_EXTENSION_NAME,
        VK_EXT_METAL_SURFACE_EXTENSION_NAME,
    ]
}

/// Translation of `UIKit_Vulkan_CreateSurface()`.
pub(crate) fn vulkan_create_surface(
    window: WindowID,
    instance: usize,
    allocator: usize,
) -> Result<u64> {
    // Note (upstream): the functions are looked up through
    // vulkan_config.vkGetInstanceProcAddr before the check that a library
    // is loaded (a NULL function called without one); here the check
    // comes first.
    let Some(get) = config().as_ref().map(|c| c.vk_get_instance_proc_addr) else {
        return Err(Error::new("Vulkan is not loaded"));
    };
    let instance = instance as *mut c_void;
    let allocator = allocator as *const c_void;
    // SAFETY: the loader's vkGetInstanceProcAddr and the caller's instance;
    // the functions have these types.
    let (vk_create_metal_surface_ext, vk_create_ios_surface_mvk) = unsafe {
        (
            instance_function::<PfnVkCreateMetalSurfaceEXT>(
                get,
                instance,
                c"vkCreateMetalSurfaceEXT",
            ),
            instance_function::<PfnVkCreateIOSSurfaceMVK>(get, instance, c"vkCreateIOSSurfaceMVK"),
        )
    };

    if vk_create_metal_surface_ext.is_none() && vk_create_ios_surface_mvk.is_none() {
        return Err(Error::new(format!(
            "{VK_EXT_METAL_SURFACE_EXTENSION_NAME} or {VK_MVK_IOS_SURFACE_EXTENSION_NAME} extensions are not enabled in the Vulkan instance."
        )));
    }

    // (UIKit_Metal_CreateView() sets the error: SDL_OutOfMemory())
    let Some(metalview) = metalview::metal_create_view(window) else {
        return Err(Error::out_of_memory());
    };

    let mut surface: u64 = 0;
    if let Some(vk_create_metal_surface_ext) = vk_create_metal_surface_ext {
        let create_info = VkMetalSurfaceCreateInfoEXT {
            s_type: VK_STRUCTURE_TYPE_METAL_SURFACE_CREATE_INFO_EXT,
            p_next: std::ptr::null(),
            flags: 0,
            p_layer: metalview::metal_get_layer(metalview).unwrap_or(0) as *const c_void,
        };
        // SAFETY: the caller's instance and allocator, the view's layer.
        let result =
            unsafe { vk_create_metal_surface_ext(instance, &create_info, allocator, &mut surface) };
        if result != VK_SUCCESS {
            metalview::metal_destroy_view(metalview);
            return Err(Error::new(format!(
                "vkCreateMetalSurfaceEXT failed: {}",
                vulkan_get_result_string(result)
            )));
        }
    } else if let Some(vk_create_ios_surface_mvk) = vk_create_ios_surface_mvk {
        let create_info = VkIOSSurfaceCreateInfoMVK {
            s_type: VK_STRUCTURE_TYPE_IOS_SURFACE_CREATE_INFO_MVK,
            p_next: std::ptr::null(),
            flags: 0,
            p_view: metalview as *const c_void,
        };
        // SAFETY: the caller's instance and allocator, the view.
        let result =
            unsafe { vk_create_ios_surface_mvk(instance, &create_info, allocator, &mut surface) };
        if result != VK_SUCCESS {
            metalview::metal_destroy_view(metalview);
            return Err(Error::new(format!(
                "vkCreateIOSSurfaceMVK failed: {}",
                vulkan_get_result_string(result)
            )));
        }
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

    Ok(surface)
}

/// Translation of `UIKit_Vulkan_DestroySurface()`.
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
