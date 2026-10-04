// Rust translation of the Vulkan and Metal parts of src/video/SDL_video.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Vulkan and Metal support: loading the Vulkan loader, the instance
//! extensions a window needs, surfaces; Metal views. The handles are the
//! raw Vulkan/Metal handles. No video driver implements these yet.

use crate::error::{Error, Result};
use crate::events::window::WindowFlags;

use super::core::{dll_not_supported, driver, uninitialized_video, with_device, with_window};
use super::window::Window;

/// Load the Vulkan loader library (`None`: the default one); calls are
/// reference counted. Translation of `SDL_Vulkan_LoadLibrary()`.
pub fn vulkan_load_library(path: Option<&str>) -> Result<()> {
    let (loaded, loaded_path) =
        with_device(|v| (v.vulkan_loader_loaded, v.vulkan_loader_path.clone()))?;
    if loaded != 0 {
        if let Some(path) = path {
            if Some(path) != loaded_path.as_deref() {
                return Err(Error::new("Vulkan loader library already loaded"));
            }
        }
    } else {
        match driver()?.vulkan_load_library(path) {
            None => return Err(dll_not_supported("Vulkan")),
            Some(result) => result?,
        }
    }
    with_device(|v| {
        v.vulkan_loader_loaded += 1;
        if v.vulkan_loader_path.is_none() {
            v.vulkan_loader_path = path.map(str::to_owned);
        }
    })
}

/// The address of `vkGetInstanceProcAddr`. Translation of
/// `SDL_Vulkan_GetVkGetInstanceProcAddr()`.
pub fn vulkan_get_vk_get_instance_proc_addr() -> Result<usize> {
    if with_device(|v| v.vulkan_loader_loaded)? == 0 {
        return Err(Error::new("No Vulkan loader has been loaded"));
    }
    driver()?
        .vulkan_get_instance_proc_addr()
        .ok_or_else(|| Error::new("No Vulkan loader has been loaded"))
}

/// Unload the Vulkan loader (when the last reference goes). Translation of
/// `SDL_Vulkan_UnloadLibrary()`.
pub fn vulkan_unload_library() {
    let Ok(Some(driver)) = with_device(|v| {
        if v.vulkan_loader_loaded > 0 {
            v.vulkan_loader_loaded -= 1;
            if v.vulkan_loader_loaded > 0 {
                return None;
            }
            v.vulkan_loader_path = None;
            Some(v.driver.clone())
        } else {
            None
        }
    }) else {
        return;
    };
    let _ = driver.vulkan_unload_library();
}

/// The Vulkan instance extensions needed for window surfaces.
/// Translation of `SDL_Vulkan_GetInstanceExtensions()`.
pub fn vulkan_instance_extensions() -> Result<Vec<&'static str>> {
    driver()?
        .vulkan_instance_extensions()
        .ok_or_else(Error::unsupported)
}

/// Create a Vulkan rendering surface for a window (converting the window
/// to a Vulkan window if needed). Translation of `SDL_Vulkan_CreateSurface()`.
pub fn vulkan_create_surface(window: &Window, instance: usize, allocator: usize) -> Result<u64> {
    let flags = window.flags()?;
    let driver = driver()?;
    if !driver.implements_vulkan_surfaces() {
        return Err(Error::unsupported());
    }

    if instance == 0 {
        return Err(Error::invalid_param("instance"));
    }

    if !flags.contains(WindowFlags::VULKAN) {
        // No problem, we can convert to Vulkan
        if flags.contains(WindowFlags::OPENGL) {
            with_window(window.id(), |w| w.core.flags &= !WindowFlags::OPENGL)?;
            super::gl::gl_unload_library();
        }
        if flags.contains(WindowFlags::METAL) {
            with_window(window.id(), |w| w.core.flags &= !WindowFlags::METAL)?;
            // Nothing more to do for Metal.
        }
        if vulkan_load_library(None).is_ok() {
            with_window(window.id(), |w| w.core.flags |= WindowFlags::VULKAN)?;
        } else {
            return Err(Error::new("failed to load Vulkan library"));
        }
    }

    driver
        .vulkan_create_surface(window.id(), instance, allocator)
        .unwrap_or_else(|| Err(Error::unsupported()))
}

/// Destroy a surface from [`vulkan_create_surface`]. Translation of
/// `SDL_Vulkan_DestroySurface()`.
pub fn vulkan_destroy_surface(instance: usize, surface: u64, allocator: usize) {
    if instance != 0 && surface != 0 {
        if let Ok(driver) = driver() {
            let _ = driver.vulkan_destroy_surface(instance, surface, allocator);
        }
    }
}

/// Whether a queue family of a physical device can present to windows.
/// Translation of `SDL_Vulkan_GetPresentationSupport()`.
pub fn vulkan_presentation_support(
    instance: usize,
    physical_device: usize,
    queue_family_index: u32,
) -> Result<bool> {
    let driver = driver().map_err(|_| uninitialized_video())?;
    if instance == 0 {
        return Err(Error::invalid_param("instance"));
    }
    if physical_device == 0 {
        return Err(Error::invalid_param("physicalDevice"));
    }

    /* If the backend does not have this function then it does not have a
     * WSI function to query it; in other words it's not necessary to check
     * as it is always supported.
     */
    Ok(driver
        .vulkan_presentation_support(instance, physical_device, queue_family_index)
        .unwrap_or(true))
}

/// Create a Metal view for a window (converting it to a Metal window if
/// needed). Translation of `SDL_Metal_CreateView()`.
pub fn metal_create_view(window: &Window) -> Result<usize> {
    let flags = window.flags()?;
    let driver = driver()?;
    if !driver.implements_metal_views() {
        return Err(Error::unsupported());
    }

    if !flags.contains(WindowFlags::METAL) {
        // No problem, we can convert to Metal
        if flags.contains(WindowFlags::OPENGL) {
            with_window(window.id(), |w| w.core.flags &= !WindowFlags::OPENGL)?;
            super::gl::gl_unload_library();
        }
        if flags.contains(WindowFlags::VULKAN) {
            with_window(window.id(), |w| w.core.flags &= !WindowFlags::VULKAN)?;
            vulkan_unload_library();
        }
        with_window(window.id(), |w| w.core.flags |= WindowFlags::METAL)?;
    }

    driver
        .metal_create_view(window.id())
        .flatten()
        .ok_or_else(|| Error::new("Couldn't create Metal view"))
}

/// Destroy a view from [`metal_create_view`]. Translation of
/// `SDL_Metal_DestroyView()`.
pub fn metal_destroy_view(view: usize) {
    if view != 0 {
        if let Ok(driver) = driver() {
            let _ = driver.metal_destroy_view(view);
        }
    }
}

/// The `CAMetalLayer` of a view. Translation of `SDL_Metal_GetLayer()`.
pub fn metal_get_layer(view: usize) -> Result<usize> {
    let not_supported = || Error::new("Metal is not supported.");
    match driver() {
        Ok(driver) if driver.implements_metal_views() => {
            if view == 0 {
                return Err(Error::invalid_param("view"));
            }
            driver
                .metal_get_layer(view)
                .flatten()
                .ok_or_else(not_supported)
        }
        _ => Err(not_supported()),
    }
}
