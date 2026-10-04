// Rust translation of src/video/offscreen/SDL_offscreenvulkan.c and
// SDL_offscreenvulkan.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Vulkan for the offscreen driver: the Vulkan loader, loaded at run time,
//! and window surfaces from `VK_EXT_headless_surface`, which present
//! nowhere (so the Vulkan renderer, say, can run without a display).
//!
//! The statically linked Vulkan Portability case of Apple platforms
//! (looking `vkGetInstanceProcAddr` up in the process with `dlsym()`) has
//! no counterpart: there is no Apple platform layer.

use std::ffi::c_void;
use std::sync::Mutex;

use crate::error::{Error, Result};
use crate::hints;
use crate::loadso::SharedObject;
use crate::video::vulkan_utils::{
    instance_function, vulkan_create_instance_extensions_list, vulkan_destroy_surface_internal,
    vulkan_get_result_string, PfnVkEnumerateInstanceExtensionProperties, PfnVkGetInstanceProcAddr,
    VkResult, VK_KHR_SURFACE_EXTENSION_NAME, VK_SUCCESS,
};

/// `VK_EXT_HEADLESS_SURFACE_EXTENSION_NAME`
const VK_EXT_HEADLESS_SURFACE_EXTENSION_NAME: &str = "VK_EXT_headless_surface";
const VK_STRUCTURE_TYPE_HEADLESS_SURFACE_CREATE_INFO_EXT: i32 = 1000256000;

/// Translation of `s_defaultPaths`.
#[cfg(windows)]
const DEFAULT_PATHS: &[&str] = &["vulkan-1.dll"];
#[cfg(target_vendor = "apple")]
const DEFAULT_PATHS: &[&str] = &[
    "vulkan.framework/vulkan",
    "libvulkan.1.dylib",
    "libvulkan.dylib",
    "MoltenVK.framework/MoltenVK",
    "libMoltenVK.dylib",
];
#[cfg(target_os = "openbsd")]
const DEFAULT_PATHS: &[&str] = &["libvulkan.so"];
#[cfg(not(any(windows, target_vendor = "apple", target_os = "openbsd")))]
const DEFAULT_PATHS: &[&str] = &["libvulkan.so.1"];

/*Should the whole driver fail if it can't create a surface? Rendering to an offscreen buffer is still possible without a surface.
At the time of writing. I need the driver to minimally work even if the surface extension isn't present.
And account for the inability to create a surface on the consumer side.
So for now I'm targeting my specific use case -Dave Kircher*/
const HEADLESS_SURFACE_EXTENSION_REQUIRED_TO_LOAD: bool = false;

/// `VkHeadlessSurfaceCreateInfoEXT`.
#[repr(C)]
struct VkHeadlessSurfaceCreateInfoEXT {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
}

type PfnVkCreateHeadlessSurfaceEXT = unsafe extern "system" fn(
    *mut c_void,
    *const VkHeadlessSurfaceCreateInfoEXT,
    *const c_void,
    *mut u64,
) -> VkResult;

/// The loader (`_this->vulkan_config`: `loader_handle`,
/// `vkGetInstanceProcAddr` and `vkEnumerateInstanceExtensionProperties`).
#[derive(Default)]
pub(super) struct VulkanConfig {
    loader_handle: Option<SharedObject>,
    vk_get_instance_proc_addr: Option<PfnVkGetInstanceProcAddr>,
    vk_enumerate_instance_extension_properties: Option<PfnVkEnumerateInstanceExtensionProperties>,
}

fn lock(config: &Mutex<VulkanConfig>) -> std::sync::MutexGuard<'_, VulkanConfig> {
    config.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `OFFSCREEN_Vulkan_LoadLibrary()`.
pub(super) fn offscreen_vulkan_load_library(
    config: &Mutex<VulkanConfig>,
    path: Option<&str>,
) -> Result<()> {
    let mut has_surface_extension = false;
    let mut has_headless_surface_extension = false;

    if lock(config).loader_handle.is_some() {
        return Err(Error::new("Vulkan already loaded"));
    }

    // Load the Vulkan loader library
    let hint = match path {
        Some(_) => None,
        None => hints::get(hints::VULKAN_LIBRARY),
    };
    let paths: Vec<&str> = match path.or(hint.as_deref()) {
        Some(path) => vec![path],
        None => DEFAULT_PATHS.to_vec(),
    };

    let Some(loader_handle) = paths.iter().find_map(|p| SharedObject::load(p).ok()) else {
        return Err(Error::new("Failed to load Vulkan Portability library"));
    };

    // (the video core keeps vulkan_config.loader_path)
    // SAFETY: vkGetInstanceProcAddr has this signature.
    let Ok(vk_get_instance_proc_addr) =
        (unsafe { loader_handle.function::<PfnVkGetInstanceProcAddr>("vkGetInstanceProcAddr") })
    else {
        // (fail: dropping the handle unloads the library)
        return Err(Error::new(
            "Failed to load vkGetInstanceProcAddr from Vulkan Portability library",
        ));
    };

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
        } else if name == VK_EXT_HEADLESS_SURFACE_EXTENSION_NAME {
            has_headless_surface_extension = true;
        }
    }
    if !has_surface_extension {
        return Err(Error::new(format!(
            "Installed Vulkan doesn't implement the {VK_KHR_SURFACE_EXTENSION_NAME} extension"
        )));
    }
    if !has_headless_surface_extension {
        if HEADLESS_SURFACE_EXTENSION_REQUIRED_TO_LOAD {
            return Err(Error::new(format!(
                "Installed Vulkan doesn't implement the {VK_EXT_HEADLESS_SURFACE_EXTENSION_NAME} extension"
            )));
        } else {
            // Let's at least leave a breadcrumb for people to find if they have issues
            crate::log::log_app!(
                "Installed Vulkan doesn't implement the {} extension",
                VK_EXT_HEADLESS_SURFACE_EXTENSION_NAME
            );
        }
    }

    let mut config = lock(config);
    config.loader_handle = Some(loader_handle);
    config.vk_get_instance_proc_addr = Some(vk_get_instance_proc_addr);
    config.vk_enumerate_instance_extension_properties =
        Some(vk_enumerate_instance_extension_properties);
    Ok(())
}

/// Translation of `OFFSCREEN_Vulkan_UnloadLibrary()`.
///
/// Note (upstream): only the loader handle is cleared there, so
/// `OFFSCREEN_Vulkan_GetInstanceExtensions()` after an unload calls the
/// `vkEnumerateInstanceExtensionProperties` of the unloaded library; here
/// the function pointers go with the library.
pub(super) fn offscreen_vulkan_unload_library(config: &Mutex<VulkanConfig>) {
    let old = std::mem::take(&mut *lock(config));
    // (dropping the handle unloads the library)
    drop(old);
}

/// The loader's `vkGetInstanceProcAddr` (`vulkan_config.vkGetInstanceProcAddr`).
pub(super) fn offscreen_vulkan_get_instance_proc_addr(
    config: &Mutex<VulkanConfig>,
) -> Option<usize> {
    lock(config).vk_get_instance_proc_addr.map(|f| f as usize)
}

/// Translation of `OFFSCREEN_Vulkan_GetInstanceExtensions()`.
pub(super) fn offscreen_vulkan_get_instance_extensions(
    config: &Mutex<VulkanConfig>,
) -> Vec<&'static str> {
    let mut return_extensions = vec![
        VK_KHR_SURFACE_EXTENSION_NAME,
        VK_EXT_HEADLESS_SURFACE_EXTENSION_NAME,
    ];

    if !HEADLESS_SURFACE_EXTENSION_REQUIRED_TO_LOAD {
        let mut has_headless_surface_extension = false;

        /* In optional mode, only return VK_EXT_HEADLESS_SURFACE_EXTENSION_NAME if it's already supported by the instance
        There's probably a better way to cache the presence of the extension during OFFSCREEN_Vulkan_LoadLibrary().
        But both SDL_VideoData and SDL_VideoDevice::vulkan_config seem like I'd need to touch a bunch of code to do properly.
        And I want a smaller footprint for the first pass*/
        if let Some(f) = lock(config).vk_enumerate_instance_extension_properties {
            if let Ok(enumerate_extensions) = vulkan_create_instance_extensions_list(f) {
                for extension in &enumerate_extensions {
                    if extension.name() == VK_EXT_HEADLESS_SURFACE_EXTENSION_NAME {
                        has_headless_surface_extension = true;
                    }
                }
            }
        }

        if !has_headless_surface_extension {
            return_extensions.pop(); // assumes VK_EXT_HEADLESS_SURFACE_EXTENSION_NAME is last
        }
    }

    return_extensions
}

/// Translation of `OFFSCREEN_Vulkan_CreateSurface()`.
pub(super) fn offscreen_vulkan_create_surface(
    config: &Mutex<VulkanConfig>,
    instance: usize,
    allocator: usize,
) -> Result<u64> {
    let mut surface: u64 = 0;

    let (loaded, get) = {
        let config = lock(config);
        (
            config.loader_handle.is_some(),
            config.vk_get_instance_proc_addr,
        )
    };
    let (true, Some(vk_get_instance_proc_addr)) = (loaded, get) else {
        return Err(Error::new("Vulkan is not loaded"));
    };

    let instance = instance as *mut c_void;
    // SAFETY: the loader's vkGetInstanceProcAddr and the caller's
    // instance; the function's type.
    let vk_create_headless_surface_ext: Option<PfnVkCreateHeadlessSurfaceEXT> = unsafe {
        instance_function(
            vk_get_instance_proc_addr,
            instance,
            c"vkCreateHeadlessSurfaceEXT",
        )
    };
    let Some(vk_create_headless_surface_ext) = vk_create_headless_surface_ext else {
        /* This may be surprising to the consumer when HEADLESS_SURFACE_EXTENSION_REQUIRED_TO_LOAD == 0
        But this is the tradeoff for allowing offscreen rendering to a buffer to continue working without requiring the extension during driver load */
        return Err(Error::new(format!(
            "{VK_EXT_HEADLESS_SURFACE_EXTENSION_NAME} extension is not enabled in the Vulkan instance."
        )));
    };
    let create_info = VkHeadlessSurfaceCreateInfoEXT {
        s_type: VK_STRUCTURE_TYPE_HEADLESS_SURFACE_CREATE_INFO_EXT,
        p_next: std::ptr::null(),
        flags: 0,
    };
    // SAFETY: the caller's instance and allocator; the create info is valid.
    let result = unsafe {
        vk_create_headless_surface_ext(
            instance,
            &create_info,
            allocator as *const c_void,
            &mut surface,
        )
    };
    if result != VK_SUCCESS {
        return Err(Error::new(format!(
            "vkCreateHeadlessSurfaceEXT failed: {}",
            vulkan_get_result_string(result)
        )));
    }
    Ok(surface)
}

/// Translation of `OFFSCREEN_Vulkan_DestroySurface()`.
pub(super) fn offscreen_vulkan_destroy_surface(
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
    use crate::events::window::WindowFlags;
    use crate::hints;
    use crate::init::{self, InitFlags};
    use crate::video::vulkan;
    use crate::video::window::Window;

    #[test]
    fn default_loader_names() {
        assert_eq!(super::DEFAULT_PATHS.len(), 1);
        #[cfg(windows)]
        assert_eq!(super::DEFAULT_PATHS[0], "vulkan-1.dll");
        #[cfg(all(unix, not(target_os = "openbsd")))]
        assert_eq!(super::DEFAULT_PATHS[0], "libvulkan.so.1");
    }

    /// The loader, the instance extensions, and a headless surface for a
    /// window (through an instance made here with the global functions).
    #[test]
    fn offscreen_vulkan_surfaces() {
        let _l = crate::test_support::test_lock();
        init::quit();
        hints::set(hints::VIDEO_DRIVER, "offscreen").unwrap();
        init::init(InitFlags::VIDEO).unwrap();

        let done = || {
            init::quit();
            hints::reset(hints::VIDEO_DRIVER);
        };

        // A loader that isn't there
        hints::set(hints::VULKAN_LIBRARY, "libno-such-vulkan.so").unwrap();
        let e = vulkan::vulkan_load_library(None).unwrap_err();
        hints::reset(hints::VULKAN_LIBRARY);
        assert_eq!(e.message(), "Failed to load Vulkan Portability library");
        assert!(vulkan::vulkan_get_vk_get_instance_proc_addr().is_err());

        if let Err(e) = vulkan::vulkan_load_library(None) {
            crate::test_support::skip(
                "vulkan",
                format_args!("no Vulkan loader with surface support ({})", e.message()),
            );
            return done();
        }
        let ext = vulkan::vulkan_instance_extensions().unwrap();
        assert_eq!(ext[0], "VK_KHR_surface");
        if ext.len() < 2 {
            crate::test_support::skip("vulkan", "no VK_EXT_headless_surface");
            vulkan::vulkan_unload_library();
            return done();
        }
        assert_eq!(ext, ["VK_KHR_surface", "VK_EXT_headless_surface"]);
        let get = vulkan::vulkan_get_vk_get_instance_proc_addr().unwrap();
        assert_ne!(get, 0);
        // A second load is counted, with the same path only
        vulkan::vulkan_load_library(None).unwrap();
        vulkan::vulkan_unload_library();

        let instance = test_instance::create(get, &ext);
        let Some(instance) = instance else {
            crate::test_support::skip("vulkan", "vkCreateInstance failed");
            vulkan::vulkan_unload_library();
            return done();
        };

        let window = Window::create("vulkan", 32, 24, WindowFlags::VULKAN).unwrap();
        assert!(window.flags().unwrap().contains(WindowFlags::VULKAN));
        assert!(vulkan::vulkan_create_surface(&window, 0, 0).is_err());
        let surface = vulkan::vulkan_create_surface(&window, instance.handle(), 0).unwrap();
        assert_ne!(surface, 0);
        vulkan::vulkan_destroy_surface(instance.handle(), surface, 0);
        // (no WSI function to ask: always supported)
        assert!(vulkan::vulkan_presentation_support(instance.handle(), 1, 0).unwrap());

        // An instance without the extension can't make surfaces
        let plain = test_instance::create(get, &[]).unwrap();
        let e = vulkan::vulkan_create_surface(&window, plain.handle(), 0).unwrap_err();
        assert_eq!(
            e.message(),
            "VK_EXT_headless_surface extension is not enabled in the Vulkan instance."
        );
        drop(plain);
        drop(instance);
        window.destroy();
        vulkan::vulkan_unload_library();
        done();
    }

    /// A bare Vulkan instance, made with the loader's global functions.
    mod test_instance {
        use std::ffi::{c_char, c_void, CString};

        use crate::video::vulkan_utils::{instance_function, PfnVkGetInstanceProcAddr};

        #[repr(C)]
        struct VkInstanceCreateInfo {
            s_type: i32,
            p_next: *const c_void,
            flags: u32,
            p_application_info: *const c_void,
            enabled_layer_count: u32,
            pp_enabled_layer_names: *const *const c_char,
            enabled_extension_count: u32,
            pp_enabled_extension_names: *const *const c_char,
        }
        type PfnVkCreateInstance = unsafe extern "system" fn(
            *const VkInstanceCreateInfo,
            *const c_void,
            *mut *mut c_void,
        ) -> i32;
        type PfnVkDestroyInstance = unsafe extern "system" fn(*mut c_void, *const c_void);

        pub(super) struct Instance {
            handle: *mut c_void,
            destroy: PfnVkDestroyInstance,
        }

        impl Instance {
            pub(super) fn handle(&self) -> usize {
                self.handle as usize
            }
        }

        impl Drop for Instance {
            fn drop(&mut self) {
                // SAFETY: the instance made by create().
                unsafe { (self.destroy)(self.handle, std::ptr::null()) };
            }
        }

        pub(super) fn create(get: usize, extensions: &[&str]) -> Option<Instance> {
            // SAFETY: the video core's vkGetInstanceProcAddr.
            let get: PfnVkGetInstanceProcAddr = unsafe { std::mem::transmute(get) };
            // SAFETY: the global function's type.
            let create: PfnVkCreateInstance =
                unsafe { instance_function(get, std::ptr::null_mut(), c"vkCreateInstance")? };
            let names: Vec<CString> = extensions
                .iter()
                .map(|e| CString::new(*e).unwrap())
                .collect();
            let pointers: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
            let info = VkInstanceCreateInfo {
                s_type: 1, // VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO
                p_next: std::ptr::null(),
                flags: 0,
                p_application_info: std::ptr::null(),
                enabled_layer_count: 0,
                pp_enabled_layer_names: std::ptr::null(),
                enabled_extension_count: pointers.len() as u32,
                pp_enabled_extension_names: pointers.as_ptr(),
            };
            let mut handle = std::ptr::null_mut();
            // SAFETY: the create info and its names outlive the call.
            if unsafe { create(&info, std::ptr::null(), &mut handle) } != 0 {
                return None;
            }
            // SAFETY: the instance function's type.
            let destroy: PfnVkDestroyInstance =
                unsafe { instance_function(get, handle, c"vkDestroyInstance")? };
            Some(Instance { handle, destroy })
        }
    }
}
