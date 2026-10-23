// Rust translation of the window and swapchain parts of
// src/gpu/metal/SDL_gpu_metal.m from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Claimed windows, their Metal views and `CAMetalLayer`s (from the video
//! driver: `SDL_Metal_CreateView()`, `SDL_Metal_GetLayer()`), compositions
//! (HDR ones too), present modes, frames in flight and swapchain texture
//! acquisition (`nextDrawable`).
//!
//! A claimed window's [`WindowEntry`] is in the window's properties (as
//! upstream's `MetalWindowData` pointer property, so that another device
//! sees the window is claimed) and in the renderer's claimed windows. Its
//! state is behind a mutex, never held across a wait for a fence.
//!
//! Upstream returns the same texture container for every acquisition,
//! updating its size and texture; here each acquisition hands out a
//! container of its own (not cyclable, with the drawable's texture and
//! the layer's size at that time), which the command buffer's use of it
//! doesn't outlive.
//!
//! Note (upstream): `METAL_INTERNAL_CreateSwapchain()` also fetches the
//! front end's blit pipelines for the swapchain's format, to cache them
//! early; the backend has no front-end device here, so they are made when
//! a blit first needs them.

use std::sync::atomic::AtomicI32;
use std::sync::{Arc, Mutex};

use super::commands::{MetalCommandBuffer, MetalFence};
use super::resources::{MetalTexture, TextureContainer, TextureContainerState};
use super::sys::*;
use super::tables::*;
use super::{lock, MetalRenderer};
use crate::core::apple::foundation::{available, CGSize};
use crate::core::apple::objc::{msg_send, Bool, Id, StrongPtr, NIL};
use crate::error::{Error, Result};
use crate::gpu::sysgpu::{BackendObject, BackendSwapchainTexture, MAX_FRAMES_IN_FLIGHT};
use crate::gpu::{
    PresentMode, SampleCount, SwapchainComposition, TextureCreateInfo, TextureFormat, TextureType,
    TextureUsageFlags,
};
use crate::video::vulkan::{metal_create_view, metal_destroy_view, metal_get_layer};
use crate::video::Window;

/// Translation of `WINDOW_PROPERTY_DATA`.
const WINDOW_PROPERTY_DATA: &str = "SDL.internal.gpu.metal.data";

/// A claimed window: the renderer that claimed it and its state.
#[derive(Debug)]
pub(super) struct WindowEntry {
    /// The claiming renderer (`renderer`), as its device.
    renderer: usize,
    pub(super) data: Mutex<WindowData>,
}

/// Translation of `MetalWindowData` (its texture and texture container are
/// made per acquisition, see the module documentation).
#[derive(Debug)]
pub(super) struct WindowData {
    pub(super) window: Window,
    pub(super) refcount: i32,
    /// The `SDL_MetalView`.
    pub(super) view: usize,
    /// The view's `CAMetalLayer` (the view keeps it).
    pub(super) layer: Id,
    pub(super) present_mode: PresentMode,
    pub(super) drawable: StrongPtr,
    /// The swapchain textures' format (`textureContainer.header.info.format`).
    pub(super) format: TextureFormat,
    pub(super) in_flight_fences: [Option<Arc<MetalFence>>; MAX_FRAMES_IN_FLIGHT as usize],
    pub(super) frame_counter: u32,
}

// SAFETY: the layer is a pointer the view keeps alive; the window data is
// used under its mutex, as upstream uses it from the threads that submit.
unsafe impl Send for WindowData {}

/// Translation of `METAL_INTERNAL_FetchWindowData()`.
fn fetch_window_data(window: Window) -> Option<Arc<WindowEntry>> {
    let properties = window.properties().ok()?;
    properties
        .get_any::<Arc<WindowEntry>>(WINDOW_PROPERTY_DATA)
        .map(|entry| (*entry).clone())
}

/// `layer.drawableSize`.
fn drawable_size(layer: Id) -> CGSize {
    // SAFETY: a CAMetalLayer property (zero on nil).
    unsafe { msg_send![layer, drawableSize] }
}

impl MetalRenderer {
    /// This renderer, as a claimed window's `renderer`.
    fn window_owner(&self) -> usize {
        self.device.get() as usize
    }

    /// Translation of `METAL_SupportsSwapchainComposition()`.
    pub(super) fn supports_swapchain_composition_internal(
        &self,
        _window: Window,
        swapchain_composition: SwapchainComposition,
    ) -> bool {
        if available!(macos(11, 0)) {
            true
        } else {
            swapchain_composition != SwapchainComposition::Hdr10St2084
        }
    }

    /// The layer settings of a composition and present mode, shared by
    /// `METAL_INTERNAL_CreateSwapchain()` and `METAL_SetSwapchainParameters()`.
    fn apply_swapchain_parameters(
        window_data: &mut WindowData,
        swapchain_composition: SwapchainComposition,
        present_mode: PresentMode,
    ) {
        let layer = window_data.layer;
        // SAFETY: CAMetalLayer properties (no-ops on nil), and a color space
        // created and released around its use.
        unsafe {
            // (SDL_PLATFORM_MACOS)
            if available!(macos(10, 13)) {
                let _: () = msg_send![layer, setDisplaySyncEnabled: Bool::from(present_mode != PresentMode::Immediate)];
                window_data.present_mode = present_mode;
            }
            let format = swapchain_composition_to_format(swapchain_composition);
            let _: () = msg_send![layer, setPixelFormat: sdl_to_metal_texture_format(format)];
            // (@available(iOS 16.0, *): always on macOS)
            let _: () = msg_send![layer, setWantsExtendedDynamicRangeContent: Bool::from(
                swapchain_composition != SwapchainComposition::Sdr
            )];

            let colorspace = CGColorSpaceCreateWithName(swapchain_composition_to_color_space(
                swapchain_composition,
            ));
            let _: () = msg_send![layer, setColorspace: colorspace];
            if !colorspace.is_null() {
                CGColorSpaceRelease(colorspace);
            }
        }
    }

    /// Translation of `METAL_INTERNAL_CreateSwapchain()`.
    ///
    /// This function assumes that it's called from within an autorelease pool
    fn create_swapchain(
        &self,
        window: Window,
        swapchain_composition: SwapchainComposition,
        present_mode: PresentMode,
    ) -> WindowData {
        // Note (upstream): C doesn't check the view or the layer; without
        // them the layer is nil and its messages do nothing.
        let view = metal_create_view(&window).unwrap_or(0);
        let layer = metal_get_layer(view).unwrap_or(0) as Id;

        let mut window_data = WindowData {
            window,
            refcount: 1,
            view,
            layer,
            present_mode: PresentMode::Vsync,
            drawable: StrongPtr::nil(),
            format: swapchain_composition_to_format(swapchain_composition),
            in_flight_fences: Default::default(),
            frame_counter: 0,
        };

        // SAFETY: a CAMetalLayer property.
        let _: () = unsafe { msg_send![layer, setDevice: self.device.get()] };
        Self::apply_swapchain_parameters(&mut window_data, swapchain_composition, present_mode);

        window_data
    }

    /// Translation of `METAL_SupportsPresentMode()`.
    pub(super) fn supports_present_mode_internal(
        &self,
        _window: Window,
        present_mode: PresentMode,
    ) -> bool {
        match present_mode {
            // (SDL_PLATFORM_MACOS)
            PresentMode::Immediate | PresentMode::Vsync => true,
            _ => false,
        }
    }

    /// Translation of `METAL_ClaimWindow()`.
    pub(super) fn claim_window_internal(&self, window: Window) -> Result<()> {
        match fetch_window_data(window) {
            None => {
                let window_data =
                    self.create_swapchain(window, SwapchainComposition::Sdr, PresentMode::Vsync);
                let entry = Arc::new(WindowEntry {
                    renderer: self.window_owner(),
                    data: Mutex::new(window_data),
                });
                window
                    .properties()?
                    .set_any(WINDOW_PROPERTY_DATA, entry.clone())?;

                lock(&self.claimed_windows).push(entry);

                Ok(())
            }
            Some(entry) if entry.renderer == self.window_owner() => {
                lock(&entry.data).refcount += 1;
                Ok(())
            }
            Some(_) => Err(self.set_string_error("Window already claimed")),
        }
    }

    /// Translation of `METAL_ReleaseWindow()`.
    pub(super) fn release_window_internal(&self, window: Window) {
        let Some(entry) = fetch_window_data(window) else {
            return;
        };
        if entry.renderer != self.window_owner() {
            // (SDL_SetError(): ReleaseWindow reports nothing)
            let _ = Error::new("Window not claimed by this device");
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

        let (view, fences) = {
            let mut window_data = lock(&entry.data);
            let fences: Vec<_> = window_data
                .in_flight_fences
                .iter_mut()
                .filter_map(Option::take)
                .collect();
            (window_data.view, fences)
        };
        metal_destroy_view(view);
        for fence in &fences {
            self.release_fence_internal(fence);
        }

        {
            let mut claimed_windows = lock(&self.claimed_windows);
            if let Some(i) = claimed_windows
                .iter()
                .position(|e| lock(&e.data).window == window)
            {
                claimed_windows.swap_remove(i);
            }
        }

        {
            let mut window_data = lock(&entry.data);
            window_data.layer = NIL;
            window_data.drawable = StrongPtr::nil();
        }

        if let Ok(properties) = window.properties() {
            properties.remove(WINDOW_PROPERTY_DATA);
        }
    }

    /// Release every claimed window, most recent first (the loop of
    /// `METAL_DestroyDevice()`).
    pub(super) fn release_claimed_windows(&self) {
        loop {
            let last = lock(&self.claimed_windows)
                .last()
                .map(|e| lock(&e.data).window);
            match last {
                Some(window) => {
                    let before = lock(&self.claimed_windows).len();
                    self.release_window_internal(window);
                    if lock(&self.claimed_windows).len() == before {
                        // (a window whose data is gone from its
                        // properties: drop it from the list)
                        lock(&self.claimed_windows).pop();
                    }
                }
                None => break,
            }
        }
    }

    /// The fence of the current frame of a claimed window, if any.
    fn current_frame_fence(entry: &WindowEntry) -> Option<Arc<MetalFence>> {
        let window_data = lock(&entry.data);
        window_data.in_flight_fences[window_data.frame_counter as usize].clone()
    }

    /// Translation of `METAL_WaitForSwapchain()`.
    pub(super) fn wait_for_swapchain_internal(&self, window: Window) -> Result<()> {
        let Some(entry) = fetch_window_data(window) else {
            return Err(
                self.set_string_error("Cannot wait for a swapchain from an unclaimed window!")
            );
        };

        if let Some(fence) = Self::current_frame_fence(&entry) {
            self.wait_for_fences_internal(true, std::slice::from_ref(&fence));
        }

        Ok(())
    }

    /// Translation of `METAL_INTERNAL_AcquireSwapchainTexture()`.
    pub(super) fn acquire_swapchain_texture_internal(
        &self,
        block: bool,
        command_buffer: &mut MetalCommandBuffer,
        window: Window,
    ) -> Result<Option<BackendSwapchainTexture>> {
        let Some(entry) = fetch_window_data(window) else {
            return Err(self.set_string_error("Window is not claimed by this SDL_GPUDevice"));
        };

        // Update the window size
        let size = drawable_size(lock(&entry.data).layer);
        let width = size.width as u32;
        let height = size.height as u32;

        if let Some(fence) = Self::current_frame_fence(&entry) {
            if block {
                // If we are blocking, just wait for the fence!
                self.wait_for_fences_internal(true, std::slice::from_ref(&fence));
            } else {
                // If we are not blocking and the least recent fence is not signaled,
                // return true to indicate that there is no error but rendering should be skipped.
                if Self::is_fence_busy(&fence) {
                    return Ok(None);
                }
            }

            self.release_fence_internal(&fence);

            let mut window_data = lock(&entry.data);
            let frame_counter = window_data.frame_counter as usize;
            window_data.in_flight_fences[frame_counter] = None;
        }

        // Get the drawable and its underlying texture
        let (texture, format) = {
            let mut window_data = lock(&entry.data);
            // SAFETY: the layer's next drawable (autoreleased, retained) and
            // its texture (retained); nil on nil.
            unsafe {
                window_data.drawable =
                    StrongPtr::retain(msg_send![window_data.layer, nextDrawable]);
                let texture = StrongPtr::retain(msg_send![window_data.drawable, texture]);
                (texture, window_data.format)
            }
        };

        // Set up presentation
        command_buffer.window_datas.push(entry.clone());

        // Return the swapchain texture
        let info = TextureCreateInfo {
            texture_type: TextureType::Texture2D,
            format,
            usage: TextureUsageFlags::COLOR_TARGET,
            width,
            height,
            layer_count_or_depth: 1,
            num_levels: 1,
            sample_count: SampleCount::One,
            props: None,
        };
        let texture = Arc::new(MetalTexture {
            handle: texture,
            reference_count: AtomicI32::new(0),
        });
        let container = TextureContainer {
            info: info.clone(),
            state: Mutex::new(TextureContainerState {
                active_texture: texture.clone(),
                textures: vec![texture],
                debug_name: None,
            }),
            can_be_cycled: false,
        };
        Ok(Some(BackendSwapchainTexture {
            raw: BackendObject::new(container),
            info,
            width,
            height,
        }))
    }

    /// Translation of `METAL_GetSwapchainTextureFormat()`.
    pub(super) fn swapchain_texture_format_internal(
        &self,
        window: Window,
    ) -> Result<TextureFormat> {
        match fetch_window_data(window) {
            Some(entry) => Ok(lock(&entry.data).format),
            None => {
                Err(self
                    .set_string_error("Cannot get swapchain format, window has not been claimed"))
            }
        }
    }

    /// Translation of `METAL_SetSwapchainParameters()`.
    pub(super) fn set_swapchain_parameters_internal(
        &self,
        window: Window,
        swapchain_composition: SwapchainComposition,
        present_mode: PresentMode,
    ) -> Result<()> {
        let Some(entry) = fetch_window_data(window) else {
            return Err(self.set_string_error(
                "Cannot set swapchain parameters, window has not been claimed!",
            ));
        };

        if !self.supports_swapchain_composition_internal(window, swapchain_composition) {
            return Err(self.set_string_error("Swapchain composition not supported"));
        }

        if !self.supports_present_mode_internal(window, present_mode) {
            return Err(self.set_string_error("Present mode not supported"));
        }

        let _ = self.wait_internal();

        let mut window_data = lock(&entry.data);
        window_data.present_mode = PresentMode::Vsync;
        Self::apply_swapchain_parameters(&mut window_data, swapchain_composition, present_mode);
        window_data.format = swapchain_composition_to_format(swapchain_composition);

        Ok(())
    }
}
