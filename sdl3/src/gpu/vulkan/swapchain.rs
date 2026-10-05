// Rust translation of the window and swapchain parts of
// src/gpu/vulkan/SDL_gpu_vulkan.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Claimed windows, their surfaces (made by the video driver's Vulkan
//! support: X11, Wayland, Windows, or the offscreen driver's headless
//! surfaces) and swapchains, swapchain texture acquisition and the
//! swapchain parameters.
//!
//! A claimed window's [`WindowEntry`] is in the window's properties (as
//! upstream's `WindowData` pointer property, so that another device sees
//! the window is claimed) and in the renderer's claimed windows. Its state
//! is behind a mutex: the acquisition, the submission's presentation and
//! the resize watch all update it (upstream doesn't lock it). The lock is
//! never held across a wait for the device or a fence, which take
//! `submitLock` (the submission takes the window's lock under it).

use std::ptr::null;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, Weak};

use super::commands::VulkanFenceHandle;
use super::resources::{
    PresentData, TextureContainer, TextureContainerState, TextureSubresource, VulkanCommandBuffer,
    VulkanTexture,
};
use super::tables::{
    sdl_to_vk_present_mode, swapchain_composition_to_sdl_format, SWAPCHAIN_COMPOSITION_SWIZZLE,
    SWAPCHAIN_COMPOSITION_TO_COLOR_SPACE, SWAPCHAIN_COMPOSITION_TO_FALLBACK_FORMAT,
    SWAPCHAIN_COMPOSITION_TO_FORMAT,
};
use super::{lock, VulkanRenderer};
use crate::error::Result;
use crate::events::window::{add_window_event_watch, WindowEventWatchPriority, WindowFlags};
use crate::events::{Event, EventType, EventWatch};
use crate::gpu::sysgpu::{BackendObject, BackendSwapchainTexture, MAX_FRAMES_IN_FLIGHT};
use crate::gpu::{
    PresentMode, SampleCount, SwapchainComposition, TextureCreateInfo, TextureFormat, TextureType,
    TextureUsageFlags,
};
use crate::log::Category;
use crate::video::vk::*;
use crate::video::Window;

/// The window property with a claimed window's data. Translation of
/// `WINDOW_PROPERTY_DATA`.
const WINDOW_PROPERTY_DATA: &str = "SDL.internal.gpu.vulkan.data";

/// A claimed window: the renderer that claimed it, its state and the watch
/// of its size.
pub(super) struct WindowEntry {
    /// The claiming renderer (`renderer`), as its instance.
    renderer: usize,
    pub(super) data: Mutex<WindowData>,
    /// `SDL_AddWindowEventWatch(VULKAN_INTERNAL_OnWindowResize)`.
    watch: Mutex<Option<EventWatch>>,
}

impl std::fmt::Debug for WindowEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowEntry")
            .field("renderer", &self.renderer)
            .finish_non_exhaustive()
    }
}

impl WindowEntry {
    /// The texture container of a swapchain image.
    pub(super) fn texture_container(&self, index: u32) -> Option<Arc<TextureContainer>> {
        lock(&self.data)
            .texture_containers
            .get(index as usize)
            .cloned()
    }
}

/// Translation of `WindowData` (its `window` and `renderer` are in the
/// [`WindowEntry`] and the data).
#[derive(Debug)]
pub(super) struct WindowData {
    pub(super) window: Window,
    refcount: i32,
    swapchain_composition: SwapchainComposition,
    present_mode: PresentMode,
    pub(super) needs_swapchain_recreate: bool,
    pub(super) needs_surface_recreate: bool,
    swapchain_create_width: u32,
    swapchain_create_height: u32,

    // Window surface
    surface: VkSurfaceKHR,

    // Swapchain for window surface
    pub(super) swapchain: VkSwapchainKHR,
    format: VkFormat,
    color_space: VkColorSpaceKHR,
    swapchain_swizzle: VkComponentMapping,
    using_fallback_format: bool,

    // Swapchain images
    /// use containers so that swapchain textures can use the same API as other textures
    pub(super) texture_containers: Vec<Arc<TextureContainer>>,
    width: u32,
    height: u32,

    // Synchronization primitives
    image_available_semaphore: [VkSemaphore; MAX_FRAMES_IN_FLIGHT as usize],
    pub(super) render_finished_semaphore: Vec<VkSemaphore>,
    pub(super) in_flight_fences: [Option<Arc<VulkanFenceHandle>>; MAX_FRAMES_IN_FLIGHT as usize],

    pub(super) frame_counter: u32,
}

// SAFETY: the Vulkan handles are plain values, used under the window's
// lock.
unsafe impl Send for WindowData {}

/// Translation of `SwapchainSupportDetails`.
#[derive(Default)]
struct SwapchainSupportDetails {
    capabilities: VkSurfaceCapabilitiesKHR,
    formats: Vec<VkSurfaceFormatKHR>,
    present_modes: Vec<VkPresentModeKHR>,
}

/// What making a swapchain did (upstream's `1` and
/// `VULKAN_INTERNAL_TRY_AGAIN`; failures are errors).
///
/// It would be nice if VULKAN_INTERNAL_CreateSwapchain could return a bool.
/// Unfortunately, some Win32 NVIDIA drivers are stupid
/// and will return surface extents of (0, 0)
/// in certain edge cases, and the swapchain extents are not allowed to be 0.
/// In this case, the client probably still wants to claim the window
/// or recreate the swapchain, so we should return 2 to indicate retry.
/// -cosmonaut
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum SwapchainResult {
    Created,
    TryAgain,
}

/// Translation of `VULKAN_INTERNAL_VerifySwapSurfaceFormat()`.
fn verify_swap_surface_format(
    desired_format: VkFormat,
    desired_color_space: VkColorSpaceKHR,
    available_formats: &[VkSurfaceFormatKHR],
) -> bool {
    available_formats
        .iter()
        .any(|f| f.format == desired_format && f.color_space == desired_color_space)
}

/// Translation of `VULKAN_INTERNAL_VerifySwapPresentMode()`.
fn verify_swap_present_mode(
    present_mode: VkPresentModeKHR,
    available_present_modes: &[VkPresentModeKHR],
) -> bool {
    available_present_modes.contains(&present_mode)
}

/// `SDL_clamp()` (which, unlike `u32::clamp`, takes the minimum when it is
/// larger than the maximum).
fn sdl_clamp(x: u32, a: u32, b: u32) -> u32 {
    if x < a {
        a
    } else if x > b {
        b
    } else {
        x
    }
}

/// A claimed window's entry. Translation of
/// `VULKAN_INTERNAL_FetchWindowData()`.
fn fetch_window_data(window: Window) -> Option<Arc<WindowEntry>> {
    let properties = window.properties().ok()?;
    properties
        .get_any::<Arc<WindowEntry>>(WINDOW_PROPERTY_DATA)
        .map(|entry| (*entry).clone())
}

impl VulkanRenderer {
    /// This renderer, as a claimed window's `renderer`.
    fn window_owner(&self) -> usize {
        self.instance as usize
    }

    /// Translation of `VULKAN_INTERNAL_QuerySwapchainSupport()`.
    fn query_swapchain_support(&self, surface: VkSurfaceKHR) -> Result<SwapchainSupportDetails> {
        let physical_device = self.physical_device;
        let mut supports_present: VkBool32 = 0;

        // SAFETY: the physical device and a surface of the instance.
        unsafe {
            (self.inst.get_physical_device_surface_support_khr)(
                physical_device,
                self.queue_family_index,
                surface,
                &mut supports_present,
            )
        };

        // Initialize these in case anything fails
        let mut output_details = SwapchainSupportDetails::default();

        if supports_present == 0 {
            return Err(self.set_string_error("This surface does not support presenting!"));
        }

        // Run the device surface queries
        // SAFETY: as above, with a structure to fill.
        let result = unsafe {
            (self.inst.get_physical_device_surface_capabilities_khr)(
                physical_device,
                surface,
                &mut output_details.capabilities,
            )
        };
        self.check(result, "vkGetPhysicalDeviceSurfaceCapabilitiesKHR")?;

        if output_details.capabilities.supported_composite_alpha & VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR
            == 0
        {
            crate::log::warn!(
                Category::Gpu,
                "Opaque presentation unsupported! Expect weird transparency bugs!"
            );
        }

        let mut formats_length = 0u32;
        // SAFETY: as above, counting.
        let result = unsafe {
            (self.inst.get_physical_device_surface_formats_khr)(
                physical_device,
                surface,
                &mut formats_length,
                std::ptr::null_mut(),
            )
        };
        // (Make sure the driver didn't mess up this value: the vectors stay empty.)
        self.check(result, "vkGetPhysicalDeviceSurfaceFormatsKHR")?;

        let mut present_modes_length = 0u32;
        // SAFETY: as above, counting.
        let result = unsafe {
            (self.inst.get_physical_device_surface_present_modes_khr)(
                physical_device,
                surface,
                &mut present_modes_length,
                std::ptr::null_mut(),
            )
        };
        self.check(result, "vkGetPhysicalDeviceSurfacePresentModesKHR")?;

        // Generate the arrays, if applicable

        if formats_length != 0 {
            output_details.formats = vec![VkSurfaceFormatKHR::default(); formats_length as usize];

            // SAFETY: as above, with room for the formats.
            let result = unsafe {
                (self.inst.get_physical_device_surface_formats_khr)(
                    physical_device,
                    surface,
                    &mut formats_length,
                    output_details.formats.as_mut_ptr(),
                )
            };
            self.check(result, "vkGetPhysicalDeviceSurfaceFormatsKHR")?;
            output_details.formats.truncate(formats_length as usize);
        }

        if present_modes_length != 0 {
            output_details.present_modes = vec![0; present_modes_length as usize];

            // SAFETY: as above, with room for the present modes.
            let result = unsafe {
                (self.inst.get_physical_device_surface_present_modes_khr)(
                    physical_device,
                    surface,
                    &mut present_modes_length,
                    output_details.present_modes.as_mut_ptr(),
                )
            };
            self.check(result, "vkGetPhysicalDeviceSurfacePresentModesKHR")?;
            output_details
                .present_modes
                .truncate(present_modes_length as usize);
        }

        /* If we made it here, all the queries were successful. This does NOT
         * necessarily mean there are any supported formats or present modes!
         */
        Ok(output_details)
    }

    /// Destroy a swapchain's images' views and the semaphores. Translation
    /// of `VULKAN_INTERNAL_DestroySwapchainImage()`.
    fn destroy_swapchain_image(&self, window_data: &mut WindowData) {
        for container in window_data.texture_containers.drain(..) {
            let texture = lock(&container.state).active_texture.clone();
            let view = texture.subresources[0].render_target_views[0];
            {
                let mut dispose = lock(&self.dispose);
                self.remove_framebuffers_containing_view(&mut dispose, view);
            }
            // SAFETY: the view of a swapchain image the GPU is done with.
            unsafe { (self.dev.destroy_image_view)(self.logical_device, view, null()) };
        }

        for semaphore in &mut window_data.image_available_semaphore {
            if *semaphore != VK_NULL_HANDLE {
                // SAFETY: a semaphore of the device nothing waits on.
                unsafe { (self.dev.destroy_semaphore)(self.logical_device, *semaphore, null()) };
                *semaphore = VK_NULL_HANDLE;
            }
        }
        for semaphore in window_data.render_finished_semaphore.drain(..) {
            if semaphore != VK_NULL_HANDLE {
                // SAFETY: as above.
                unsafe { (self.dev.destroy_semaphore)(self.logical_device, semaphore, null()) };
            }
        }
    }

    /// Translation of `VULKAN_INTERNAL_DestroySwapchain()`.
    fn destroy_swapchain(&self, window_data: &mut WindowData) {
        self.destroy_swapchain_image(window_data);

        if window_data.swapchain != VK_NULL_HANDLE {
            // SAFETY: the window's swapchain, unused by the GPU.
            unsafe {
                (self.dev.destroy_swapchain_khr)(self.logical_device, window_data.swapchain, null())
            };
            window_data.swapchain = VK_NULL_HANDLE;
        }
    }

    /// Destroy the window's swapchain after a failure (the error paths of
    /// `VULKAN_INTERNAL_CreateSwapchain()`).
    fn drop_swapchain(&self, window_data: &mut WindowData) {
        // SAFETY: the swapchain just created.
        unsafe {
            (self.dev.destroy_swapchain_khr)(self.logical_device, window_data.swapchain, null())
        };
        window_data.swapchain = VK_NULL_HANDLE;
    }

    /// Make the window's swapchain (replacing the old one), its images'
    /// texture containers and the semaphores. Translation of
    /// `VULKAN_INTERNAL_CreateSwapchain()`.
    ///
    /// Note (upstream): when a later step fails, C leaves the image count
    /// and texture containers half made (which the swapchain's destruction
    /// then reads); here only the containers made are kept, to be
    /// destroyed with the swapchain.
    fn create_swapchain(&self, window_data: &mut WindowData) -> Result<SwapchainResult> {
        window_data.frame_counter = 0;

        let swapchain_support_details = self.query_swapchain_support(window_data.surface)?;

        // Verify that we can use the requested composition and present mode
        let composition = window_data.swapchain_composition as usize;
        window_data.format = SWAPCHAIN_COMPOSITION_TO_FORMAT[composition];
        window_data.color_space = SWAPCHAIN_COMPOSITION_TO_COLOR_SPACE[composition];
        window_data.swapchain_swizzle = SWAPCHAIN_COMPOSITION_SWIZZLE[composition];
        window_data.using_fallback_format = false;

        let mut has_valid_swapchain_composition = verify_swap_surface_format(
            window_data.format,
            window_data.color_space,
            &swapchain_support_details.formats,
        );

        if !has_valid_swapchain_composition {
            // Let's try again with the fallback format...
            window_data.format = SWAPCHAIN_COMPOSITION_TO_FALLBACK_FORMAT[composition];
            window_data.using_fallback_format = true;
            has_valid_swapchain_composition = verify_swap_surface_format(
                window_data.format,
                window_data.color_space,
                &swapchain_support_details.formats,
            );
        }

        let has_valid_present_mode = verify_swap_present_mode(
            sdl_to_vk_present_mode(window_data.present_mode),
            &swapchain_support_details.present_modes,
        );

        if !has_valid_swapchain_composition {
            return Err(
                self.set_string_error("Device does not support requested swapchain composition!")
            );
        }
        if !has_valid_present_mode {
            return Err(self.set_string_error("Device does not support requested present_mode!"));
        }

        let capabilities = &swapchain_support_details.capabilities;

        // NVIDIA + Win32 can return 0 extent when the window is minimized. Try again!
        if capabilities.current_extent.width == 0 || capabilities.current_extent.height == 0 {
            return Ok(SwapchainResult::TryAgain);
        }

        let mut requested_image_count = self.allowed_frames_in_flight.load(Ordering::SeqCst);

        // (SDL_PLATFORM_APPLE takes the current extent.)
        window_data.width = sdl_clamp(
            window_data.swapchain_create_width,
            capabilities.min_image_extent.width,
            capabilities.max_image_extent.width,
        );
        window_data.height = sdl_clamp(
            window_data.swapchain_create_height,
            capabilities.min_image_extent.height,
            capabilities.max_image_extent.height,
        );

        if capabilities.max_image_count > 0 && requested_image_count > capabilities.max_image_count
        {
            requested_image_count = capabilities.max_image_count;
        }

        if requested_image_count < capabilities.min_image_count {
            requested_image_count = capabilities.min_image_count;
        }

        if window_data.present_mode == PresentMode::Mailbox {
            /* Required for proper triple-buffering.
             * Note that this is below the above maxImageCount check!
             * If the driver advertises MAILBOX but does not support 3 swap
             * images, it's not real mailbox support, so let it fail hard.
             * -flibit
             */
            requested_image_count = requested_image_count.max(3);
        }

        // Default to opaque, if available, followed by inherit, and overwrite with a value that supports transparency, if necessary.
        let supported_composite_alpha = capabilities.supported_composite_alpha;
        let mut composite_alpha_flag = 0;
        if supported_composite_alpha & VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR != 0 {
            composite_alpha_flag = VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR;
        } else if supported_composite_alpha & VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR != 0 {
            composite_alpha_flag = VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR;
        }

        let transparent = window_data
            .window
            .flags()
            .is_ok_and(|f| f.contains(WindowFlags::TRANSPARENT));
        if transparent || composite_alpha_flag == 0 {
            if supported_composite_alpha & VK_COMPOSITE_ALPHA_PRE_MULTIPLIED_BIT_KHR != 0 {
                composite_alpha_flag = VK_COMPOSITE_ALPHA_PRE_MULTIPLIED_BIT_KHR;
            } else if supported_composite_alpha & VK_COMPOSITE_ALPHA_POST_MULTIPLIED_BIT_KHR != 0 {
                composite_alpha_flag = VK_COMPOSITE_ALPHA_POST_MULTIPLIED_BIT_KHR;
            } else if supported_composite_alpha & VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR != 0 {
                composite_alpha_flag = VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR;
            } else {
                crate::log::warn!(
                    Category::Gpu,
                    "SDL_WINDOW_TRANSPARENT flag set, but no suitable swapchain composite alpha value supported!"
                );
            }
        }

        let swapchain_create_info = VkSwapchainCreateInfoKHR {
            s_type: VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR,
            p_next: null(),
            flags: 0,
            surface: window_data.surface,
            min_image_count: requested_image_count,
            image_format: window_data.format,
            image_color_space: window_data.color_space,
            image_extent: VkExtent2D {
                width: window_data.width,
                height: window_data.height,
            },
            image_array_layers: 1,
            image_usage: VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT,
            image_sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: null(),
            // (SDL_PLATFORM_ANDROID takes VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR.)
            pre_transform: capabilities.current_transform,
            composite_alpha: composite_alpha_flag,
            present_mode: sdl_to_vk_present_mode(window_data.present_mode),
            clipped: VK_TRUE,
            // The old swapchain could belong to a surface that no longer exists due to app switching.
            old_swapchain: if window_data.needs_surface_recreate {
                VK_NULL_HANDLE
            } else {
                window_data.swapchain
            },
        };
        let mut swapchain = VK_NULL_HANDLE;
        // SAFETY: a valid create info, with the window's surface.
        let vulkan_result = unsafe {
            (self.dev.create_swapchain_khr)(
                self.logical_device,
                &swapchain_create_info,
                null(),
                &mut swapchain,
            )
        };
        window_data.swapchain = swapchain;

        if swapchain_create_info.old_swapchain != VK_NULL_HANDLE {
            // SAFETY: the retired swapchain, unused by the GPU.
            unsafe {
                (self.dev.destroy_swapchain_khr)(
                    self.logical_device,
                    swapchain_create_info.old_swapchain,
                    null(),
                )
            };
        }

        if vulkan_result != VK_SUCCESS {
            window_data.swapchain = VK_NULL_HANDLE;
            return Err(self.vk_error(vulkan_result, "vkCreateSwapchainKHR"));
        }

        let mut image_count = 0u32;
        // SAFETY: the new swapchain, counting.
        let vulkan_result = unsafe {
            (self.dev.get_swapchain_images_khr)(
                self.logical_device,
                window_data.swapchain,
                &mut image_count,
                std::ptr::null_mut(),
            )
        };
        self.check(vulkan_result, "vkGetSwapchainImagesKHR")?;

        let mut swapchain_images = vec![VK_NULL_HANDLE; image_count as usize];

        // SAFETY: as above, with room for the images.
        let vulkan_result = unsafe {
            (self.dev.get_swapchain_images_khr)(
                self.logical_device,
                window_data.swapchain,
                &mut image_count,
                swapchain_images.as_mut_ptr(),
            )
        };
        self.check(vulkan_result, "vkGetSwapchainImagesKHR")?;
        swapchain_images.truncate(image_count as usize);

        for &image in &swapchain_images {
            match self.create_swapchain_texture_container(window_data, image) {
                Ok(container) => window_data.texture_containers.push(container),
                Err(e) => {
                    self.drop_swapchain(window_data);
                    return Err(e);
                }
            }
        }

        let semaphore_create_info = VkSemaphoreCreateInfo {
            s_type: VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO,
            p_next: null(),
            flags: 0,
        };

        for i in 0..MAX_FRAMES_IN_FLIGHT as usize {
            let mut semaphore = VK_NULL_HANDLE;
            // SAFETY: a valid create info.
            let vulkan_result = unsafe {
                (self.dev.create_semaphore)(
                    self.logical_device,
                    &semaphore_create_info,
                    null(),
                    &mut semaphore,
                )
            };

            if vulkan_result != VK_SUCCESS {
                self.drop_swapchain(window_data);
                return Err(self.vk_error(vulkan_result, "vkCreateSemaphore"));
            }
            window_data.image_available_semaphore[i] = semaphore;

            window_data.in_flight_fences[i] = None;
        }

        for _ in 0..image_count {
            let mut semaphore = VK_NULL_HANDLE;
            // SAFETY: a valid create info.
            let vulkan_result = unsafe {
                (self.dev.create_semaphore)(
                    self.logical_device,
                    &semaphore_create_info,
                    null(),
                    &mut semaphore,
                )
            };

            if vulkan_result != VK_SUCCESS {
                self.drop_swapchain(window_data);
                return Err(self.vk_error(vulkan_result, "vkCreateSemaphore"));
            }
            window_data.render_finished_semaphore.push(semaphore);
        }

        window_data.needs_swapchain_recreate = false;
        Ok(SwapchainResult::Created)
    }

    /// The texture container of a swapchain image ("Initialize dummy
    /// container" in `VULKAN_INTERNAL_CreateSwapchain()`).
    fn create_swapchain_texture_container(
        &self,
        window_data: &WindowData,
        image: VkImage,
    ) -> Result<Arc<TextureContainer>> {
        let info = TextureCreateInfo {
            texture_type: TextureType::Texture2D,
            format: swapchain_composition_to_sdl_format(
                window_data.swapchain_composition,
                window_data.using_fallback_format,
            ),
            usage: TextureUsageFlags::COLOR_TARGET,
            width: window_data.width,
            height: window_data.height,
            layer_count_or_depth: 1,
            num_levels: 1,
            sample_count: SampleCount::One,
            props: None,
        };

        let mut texture = VulkanTexture {
            container: Mutex::new(None),
            image,
            // Swapchain memory is managed by the driver
            used_region: None,
            full_view: VK_NULL_HANDLE,
            swizzle: window_data.swapchain_swizzle,
            aspect_flags: VK_IMAGE_ASPECT_COLOR_BIT,
            depth: 1,
            usage: TextureUsageFlags::COLOR_TARGET,
            level_count: 1,
            layer_count: 1,
            ty: TextureType::Texture2D,
            // Create slice
            subresources: Vec::new(),
            marked_for_destroy: AtomicBool::new(false),
            externally_managed: false,
            reference_count: AtomicI32::new(0),
        };

        let view = self.create_render_target_view(
            &texture,
            0,
            0,
            window_data.format,
            window_data.swapchain_swizzle,
        )?;
        texture.subresources.push(TextureSubresource {
            layer: 0,
            level: 0,
            render_target_views: vec![view],
            ..Default::default()
        });

        let texture = Arc::new(texture);
        let container = Arc::new(TextureContainer {
            info,
            state: Mutex::new(TextureContainerState {
                active_texture: texture.clone(),
                textures: Vec::new(),
                debug_name: None,
            }),
            can_be_cycled: false,
            externally_managed: false,
        });
        texture.set_container(Some(super::resources::ContainerRef {
            container: Arc::downgrade(&container),
            index: 0,
        }));

        Ok(container)
    }

    /// Translation of `VULKAN_SupportsSwapchainComposition()`.
    pub(super) fn supports_swapchain_composition_internal(
        &self,
        window: Window,
        swapchain_composition: SwapchainComposition,
    ) -> bool {
        let Some(entry) = fetch_window_data(window) else {
            let _ = self.set_string_error(
                "Must claim window before querying swapchain composition support!",
            );
            return false;
        };

        let surface = lock(&entry.data).surface;
        if surface == VK_NULL_HANDLE {
            let _ = self.set_string_error("Window has no Vulkan surface");
            return false;
        }

        let Ok(support_details) = self.query_swapchain_support(surface) else {
            return false;
        };

        let composition = swapchain_composition as usize;
        verify_swap_surface_format(
            SWAPCHAIN_COMPOSITION_TO_FORMAT[composition],
            SWAPCHAIN_COMPOSITION_TO_COLOR_SPACE[composition],
            &support_details.formats,
        ) || {
            // Let's try again with the fallback format...
            verify_swap_surface_format(
                SWAPCHAIN_COMPOSITION_TO_FALLBACK_FORMAT[composition],
                SWAPCHAIN_COMPOSITION_TO_COLOR_SPACE[composition],
                &support_details.formats,
            )
        }
    }

    /// Translation of `VULKAN_SupportsPresentMode()`.
    pub(super) fn supports_present_mode_internal(
        &self,
        window: Window,
        present_mode: PresentMode,
    ) -> bool {
        let Some(entry) = fetch_window_data(window) else {
            let _ =
                self.set_string_error("Must claim window before querying present mode support!");
            return false;
        };

        let surface = lock(&entry.data).surface;
        if surface == VK_NULL_HANDLE {
            let _ = self.set_string_error("Window has no Vulkan surface");
            return false;
        }

        let Ok(support_details) = self.query_swapchain_support(surface) else {
            return false;
        };

        verify_swap_present_mode(
            sdl_to_vk_present_mode(present_mode),
            &support_details.present_modes,
        )
    }

    /// Claim a window: make its surface and swapchain. Translation of
    /// `VULKAN_ClaimWindow()`.
    pub(super) fn claim_window_internal(&self, window: Window) -> Result<()> {
        if let Some(entry) = fetch_window_data(window) {
            if entry.renderer == self.window_owner() {
                lock(&entry.data).refcount += 1;
                return Ok(());
            }
            return Err(self.set_string_error("Window already claimed"));
        }

        let mut window_data = WindowData {
            window,
            refcount: 1,
            swapchain_composition: SwapchainComposition::Sdr,
            present_mode: PresentMode::Vsync,
            needs_swapchain_recreate: false,
            needs_surface_recreate: false,
            swapchain_create_width: 0,
            swapchain_create_height: 0,
            surface: VK_NULL_HANDLE,
            swapchain: VK_NULL_HANDLE,
            format: VK_FORMAT_UNDEFINED,
            color_space: 0,
            swapchain_swizzle: VkComponentMapping::default(),
            using_fallback_format: false,
            texture_containers: Vec::new(),
            width: 0,
            height: 0,
            image_available_semaphore: [VK_NULL_HANDLE; MAX_FRAMES_IN_FLIGHT as usize],
            render_finished_semaphore: Vec::new(),
            in_flight_fences: Default::default(),
            frame_counter: 0,
        };

        // On non-Apple platforms the swapchain capability currentExtent can be different from the window,
        // so we have to query the window size.
        let _ = window.sync();
        let (w, h) = window.size_in_pixels()?;
        window_data.swapchain_create_width = w as u32;
        window_data.swapchain_create_height = h as u32;

        // Each window must have its own surface.
        // FIXME: VAllocationCallbacks
        // (SDL_Vulkan_CreateSurface() checks the video device and its
        // Vulkan_CreateSurface.)
        window_data.surface =
            crate::video::vulkan::vulkan_create_surface(&window, self.instance as usize, 0)?;

        match self.create_swapchain(&mut window_data) {
            Ok(SwapchainResult::Created) => {
                let entry = Arc::new(WindowEntry {
                    renderer: self.window_owner(),
                    data: Mutex::new(window_data),
                    watch: Mutex::new(None),
                });
                window
                    .properties()?
                    .set_any(WINDOW_PROPERTY_DATA, entry.clone())?;

                lock(&self.claimed_windows).push(entry.clone());

                *lock(&entry.watch) = Some(watch_window_resize(window, Arc::downgrade(&entry)));

                Ok(())
            }
            Ok(SwapchainResult::TryAgain) => {
                // FIXME (upstream): the window data (and its surface) is
                // lost: it is stored neither in the window nor in the
                // claimed windows, so the window isn't claimed.
                window_data.needs_swapchain_recreate = true;
                Ok(())
            }
            Err(e) => {
                // Failed to create swapchain, destroy surface and free data
                // SAFETY: the surface just made.
                unsafe {
                    (self.inst.destroy_surface_khr)(self.instance, window_data.surface, null())
                };
                Err(e)
            }
        }
    }

    /// Unclaim a window: destroy its swapchain and surface. Translation of
    /// `VULKAN_ReleaseWindow()`.
    pub(super) fn release_window_internal(&self, window: Window) {
        let Some(entry) = fetch_window_data(window) else {
            return;
        };
        if entry.renderer != self.window_owner() {
            let _ = self.set_string_error("Window not claimed by this device");
            return;
        }
        {
            let mut window_data = lock(&entry.data);
            if window_data.refcount > 1 {
                window_data.refcount -= 1;
                return;
            }
        }

        let _ = self.wait_internal();

        {
            let mut window_data = lock(&entry.data);

            for fence in &mut window_data.in_flight_fences {
                if let Some(fence) = fence.take() {
                    self.release_fence_internal(&fence);
                }
            }

            self.destroy_swapchain(&mut window_data);

            // SAFETY: the window's surface, without a swapchain.
            unsafe { (self.inst.destroy_surface_khr)(self.instance, window_data.surface, null()) };
            window_data.surface = VK_NULL_HANDLE;
        }

        {
            let mut claimed_windows = lock(&self.claimed_windows);
            if let Some(i) = claimed_windows.iter().position(|e| Arc::ptr_eq(e, &entry)) {
                claimed_windows.swap_remove(i);
            }
        }

        if let Ok(properties) = window.properties() {
            properties.remove(WINDOW_PROPERTY_DATA);
        }
        // SDL_RemoveWindowEventWatch()
        lock(&entry.watch).take();
    }

    /// Wait for the device, then remake a window's swapchain. Translation of
    /// `VULKAN_INTERNAL_RecreateSwapchain()` (the window's lock is taken
    /// after the wait).
    fn recreate_swapchain(&self, entry: &WindowEntry) -> Result<SwapchainResult> {
        self.wait_internal()?;

        let mut window_data = lock(&entry.data);

        for fence in &mut window_data.in_flight_fences {
            if let Some(fence) = fence.take() {
                self.release_fence_internal(&fence);
            }
        }

        // (SDL_VIDEO_DRIVER_PRIVATE destroys the swapchain too.)
        self.destroy_swapchain_image(&mut window_data);
        self.create_swapchain(&mut window_data)
    }

    /// Block until the window's next swapchain texture can be acquired.
    /// Translation of `VULKAN_WaitForSwapchain()`.
    pub(super) fn wait_for_swapchain_internal(&self, window: Window) -> Result<()> {
        let Some(entry) = fetch_window_data(window) else {
            return Err(
                self.set_string_error("Cannot wait for a swapchain from an unclaimed window!")
            );
        };

        let fence = {
            let window_data = lock(&entry.data);
            window_data.in_flight_fences[window_data.frame_counter as usize].clone()
        };
        if let Some(fence) = fence {
            self.wait_for_fences_internal(true, &[&fence])?;
        }

        Ok(())
    }

    /// Acquire the window's next swapchain image for a command buffer,
    /// waiting for its frame's fence if `block`. Translation of
    /// `VULKAN_INTERNAL_AcquireSwapchainTexture()`.
    pub(super) fn acquire_swapchain_texture_internal(
        &self,
        block: bool,
        vulkan_command_buffer: &mut VulkanCommandBuffer,
        window: Window,
    ) -> Result<Option<BackendSwapchainTexture>> {
        let Some(entry) = fetch_window_data(window) else {
            return Err(self
                .set_string_error("Cannot acquire a swapchain texture from an unclaimed window!"));
        };

        // The command buffer is flagged for cleanup when the swapchain is requested as a cleanup timing mechanism
        vulkan_command_buffer.swapchain_requested = true;

        if window
            .flags()
            .is_ok_and(|f| f.contains(WindowFlags::HIDDEN))
        {
            // Edge case, texture is filled in with NULL but not an error
            return Ok(None);
        }

        let mut window_data = lock(&entry.data);

        if window_data.needs_surface_recreate {
            // SAFETY: the window's old surface.
            unsafe { (self.inst.destroy_surface_khr)(self.instance, window_data.surface, null()) };
            // FIXME: VAllocationCallbacks
            match crate::video::vulkan::vulkan_create_surface(&window, self.instance as usize, 0) {
                Ok(surface) => window_data.surface = surface,
                Err(_) => return Err(self.set_string_error("Failed to recreate Vulkan surface!")),
            }
        }

        // If window data marked as needing swapchain recreate, try to recreate
        if window_data.needs_swapchain_recreate {
            drop(window_data);
            let recreate_swapchain_result = self.recreate_swapchain(&entry)?;
            window_data = lock(&entry.data);
            if recreate_swapchain_result == SwapchainResult::TryAgain {
                // Edge case, texture is filled in with NULL but not an error
                let frame_counter = window_data.frame_counter as usize;
                if let Some(fence) = window_data.in_flight_fences[frame_counter].take() {
                    self.release_fence_internal(&fence);
                }
                return Ok(None);
            }

            // Unset this flag until after the swapchain has been recreated to let VULKAN_INTERNAL_CreateSwapchain()
            // know whether it needs to pass the old swapchain or not.
            window_data.needs_surface_recreate = false;
        }

        let frame_counter = window_data.frame_counter as usize;
        if let Some(fence) = window_data.in_flight_fences[frame_counter].clone() {
            if block {
                // If we are blocking, just wait for the fence!
                drop(window_data);
                self.wait_for_fences_internal(true, &[&fence])?;
                window_data = lock(&entry.data);
            } else {
                // If we are not blocking and the least recent fence is not signaled,
                // return true to indicate that there is no error but rendering should be skipped.
                if !self.query_fence_internal(&fence) {
                    return Ok(None);
                }
            }

            self.release_fence_internal(&fence);

            window_data.in_flight_fences[frame_counter] = None;
        }

        // Finally, try to acquire!
        let mut swapchain_image_index = 0u32;
        loop {
            // SAFETY: the window's swapchain and the frame's semaphore.
            let acquire_result = unsafe {
                (self.dev.acquire_next_image_khr)(
                    self.logical_device,
                    window_data.swapchain,
                    u64::MAX,
                    window_data.image_available_semaphore[window_data.frame_counter as usize],
                    VK_NULL_HANDLE,
                    &mut swapchain_image_index,
                )
            };

            if acquire_result == VK_SUCCESS || acquire_result == VK_SUBOPTIMAL_KHR {
                break; // we got the next image!
            }

            // Surface lost — flag for surface + swapchain recreation on next call
            if acquire_result == VK_ERROR_SURFACE_LOST_KHR {
                window_data.needs_surface_recreate = true;
                window_data.needs_swapchain_recreate = true;
                return Ok(None);
            }

            // If acquisition is invalid, let's try to recreate
            drop(window_data);
            let recreate_swapchain_result = self.recreate_swapchain(&entry)?;
            window_data = lock(&entry.data);
            if recreate_swapchain_result == SwapchainResult::TryAgain {
                // Edge case, texture is filled in with NULL but not an error
                return Ok(None);
            }
        }

        let swapchain_texture_container =
            window_data.texture_containers[swapchain_image_index as usize].clone();

        // We need a special execution dependency with pWaitDstStageMask or image transition can start before acquire finishes

        let image_barrier = VkImageMemoryBarrier {
            s_type: VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
            p_next: null(),
            src_access_mask: 0,
            dst_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image: lock(&swapchain_texture_container.state)
                .active_texture
                .image,
            subresource_range: VkImageSubresourceRange {
                aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            },
        };

        // SAFETY: a command buffer being recorded and the acquired image.
        unsafe {
            (self.dev.cmd_pipeline_barrier)(
                vulkan_command_buffer.command_buffer,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                0,
                0,
                null(),
                0,
                null(),
                1,
                &image_barrier,
            )
        };

        // Set up present struct

        vulkan_command_buffer.present_datas.push(PresentData {
            window_data: entry.clone(),
            swapchain_image_index,
        });

        // Set up present semaphores

        vulkan_command_buffer
            .wait_semaphores
            .push(window_data.image_available_semaphore[window_data.frame_counter as usize]);

        vulkan_command_buffer
            .signal_semaphores
            .push(window_data.render_finished_semaphore[swapchain_image_index as usize]);

        Ok(Some(BackendSwapchainTexture {
            raw: BackendObject(swapchain_texture_container.clone()),
            info: swapchain_texture_container.info.clone(),
            width: window_data.width,
            height: window_data.height,
        }))
    }

    /// Translation of `VULKAN_GetSwapchainTextureFormat()`.
    pub(super) fn swapchain_texture_format_internal(
        &self,
        window: Window,
    ) -> Result<TextureFormat> {
        let Some(entry) = fetch_window_data(window) else {
            return Err(
                self.set_string_error("Cannot get swapchain format, window has not been claimed!")
            );
        };

        let window_data = lock(&entry.data);
        Ok(swapchain_composition_to_sdl_format(
            window_data.swapchain_composition,
            window_data.using_fallback_format,
        ))
    }

    /// Translation of `VULKAN_SetSwapchainParameters()`.
    pub(super) fn set_swapchain_parameters_internal(
        &self,
        window: Window,
        swapchain_composition: SwapchainComposition,
        present_mode: PresentMode,
    ) -> Result<()> {
        let Some(entry) = fetch_window_data(window) else {
            return Err(
                self.set_string_error("Cannot set swapchain parameters on unclaimed window!")
            );
        };

        if !self.supports_swapchain_composition_internal(window, swapchain_composition) {
            return Err(self.set_string_error("Swapchain composition not supported!"));
        }

        if !self.supports_present_mode_internal(window, present_mode) {
            return Err(self.set_string_error("Present mode not supported!"));
        }

        {
            let mut window_data = lock(&entry.data);
            window_data.present_mode = present_mode;
            window_data.swapchain_composition = swapchain_composition;
        }

        if self.recreate_swapchain(&entry)? == SwapchainResult::TryAgain {
            // Edge case, swapchain extent is (0, 0) but this is not an error
            lock(&entry.data).needs_swapchain_recreate = true;
        }

        Ok(())
    }

    /// Translation of `VULKAN_SetAllowedFramesInFlight()`.
    pub(super) fn set_allowed_frames_in_flight_internal(
        &self,
        allowed_frames_in_flight: u32,
    ) -> Result<()> {
        self.allowed_frames_in_flight
            .store(allowed_frames_in_flight, Ordering::SeqCst);

        let claimed_windows = lock(&self.claimed_windows).clone();
        for entry in &claimed_windows {
            if self.recreate_swapchain(entry)? == SwapchainResult::TryAgain {
                // Edge case, swapchain extent is (0, 0) but this is not an error
                lock(&entry.data).needs_swapchain_recreate = true;
            }
        }

        Ok(())
    }

    /// Release every claimed window, most recent first (`VULKAN_DestroyDevice()`).
    pub(super) fn release_claimed_windows(&self) {
        let claimed_windows = lock(&self.claimed_windows).clone();
        for entry in claimed_windows.iter().rev() {
            let window = lock(&entry.data).window;
            self.release_window_internal(window);
        }
    }
}

/// Flag a window's swapchain for recreation at its new size. Translation of
/// `VULKAN_INTERNAL_OnWindowResize()`.
fn watch_window_resize(window: Window, entry: Weak<WindowEntry>) -> EventWatch {
    let window_id = window.id();
    add_window_event_watch(WindowEventWatchPriority::Normal, move |e| {
        let Event::Window(w) = e else {
            return;
        };
        if w.event_type == EventType::WINDOW_PIXEL_SIZE_CHANGED && w.window_id == window_id {
            if let Some(entry) = entry.upgrade() {
                let mut data = lock(&entry.data);
                data.needs_swapchain_recreate = true;
                data.swapchain_create_width = w.data1 as u32;
                data.swapchain_create_height = w.data2 as u32;
            }
        }
    })
}
