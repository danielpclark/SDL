// Rust translation of the window and swapchain parts of
// src/gpu/d3d12/SDL_gpu_d3d12.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Claimed windows and their DXGI swapchains (the flip model, on the
//! device's command queue), present modes, compositions (HDR ones too),
//! frames in flight, and swapchain texture acquisition and resizing. The
//! Xbox (GDK) swapchains are not translated.
//!
//! A claimed window's [`WindowEntry`] is in the window's properties (as
//! upstream's `D3D12WindowData` pointer property, so that another device
//! sees the window is claimed) and in the renderer's claimed windows. Its
//! state is behind a mutex: the acquisition, the submission's presentation
//! and the resize watch all update it (upstream doesn't lock it). The lock
//! is never held across a wait for the device or a fence, which take
//! `submitLock` (the submission takes the window's lock under it).
//!
//! Upstream releases the reference to a swapchain buffer it gets when
//! making the buffer's views, gets the buffer again when the texture is
//! acquired and releases it after presenting. Here the swapchain texture
//! keeps the reference it got first (as a texture keeps its resource),
//! until the swapchain's textures are released before its buffers are
//! resized or it is destroyed, which DXGI requires.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, Weak};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;

use super::commands::{D3D12CommandBuffer, D3D12Fence, PresentData};
use super::d3d::*;
use super::resources::{
    ContainerRef, D3D12Texture, TextureContainer, TextureContainerState, TextureSubresource,
};
use super::tables::{
    swapchain_composition_index, SWAPCHAIN_COMPOSITION_TO_COLOR_SPACE,
    SWAPCHAIN_COMPOSITION_TO_SDL_TEXTURE_FORMAT, SWAPCHAIN_COMPOSITION_TO_TEXTURE_FORMAT,
};
use super::{lock, D3D12Renderer};
use crate::error::Result;
use crate::events::window::{add_window_event_watch, WindowEventWatchPriority};
use crate::events::{Event, EventType, EventWatch};
use crate::gpu::sysgpu::{BackendObject, BackendSwapchainTexture, MAX_FRAMES_IN_FLIGHT};
use crate::gpu::{
    PresentMode, SampleCount, SwapchainComposition, TextureCreateInfo, TextureFormat, TextureType,
    TextureUsageFlags,
};
use crate::log::Category;
use crate::render::direct3d11::d3d::{
    DxgiColorSpaceType, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    DXGI_FORMAT_UNKNOWN,
};
use crate::video::window::PROP_WINDOW_WIN32_HWND_POINTER;
use crate::video::Window;

/// The window property with a claimed window's data. Translation of
/// `WINDOW_PROPERTY_DATA`.
const WINDOW_PROPERTY_DATA: &str = "SDL.internal.gpu.d3d12.data";

/// A claimed window: the renderer that claimed it, its state and the watch
/// of its size.
pub(super) struct WindowEntry {
    /// The claiming renderer (`renderer`), as its device.
    renderer: usize,
    pub(super) data: Mutex<WindowData>,
    /// `SDL_AddWindowEventWatch(D3D12_INTERNAL_OnWindowResize)`.
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

/// Translation of `D3D12WindowData` (its `renderer` is in the
/// [`WindowEntry`]).
#[derive(Debug)]
pub(super) struct WindowData {
    pub(super) window: Window,
    refcount: i32,
    pub(super) swapchain: Option<DxgiSwapChain3>,
    pub(super) present_mode: PresentMode,
    swapchain_composition: SwapchainComposition,
    #[allow(dead_code)] // (kept as upstream does)
    swapchain_color_space: DxgiColorSpaceType,
    pub(super) frame_counter: u32,

    /// One per swapchain buffer (`textureContainers`).
    pub(super) texture_containers: Vec<Arc<TextureContainer>>,
    pub(super) swapchain_texture_count: u32,

    pub(super) in_flight_fences: [Option<Arc<D3D12Fence>>; MAX_FRAMES_IN_FLIGHT as usize],
    width: u32,
    height: u32,
    needs_swapchain_recreate: bool,
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
/// `D3D12_INTERNAL_FetchWindowData()`.
fn fetch_window_data(window: Window) -> Option<Arc<WindowEntry>> {
    let properties = window.properties().ok()?;
    properties
        .get_any::<Arc<WindowEntry>>(WINDOW_PROPERTY_DATA)
        .map(|entry| (*entry).clone())
}

/// The window's `HWND` (`SDL_PROP_WINDOW_WIN32_HWND_POINTER`), NULL without
/// one.
fn window_hwnd(window: Window) -> HWND {
    window
        .properties()
        .ok()
        .and_then(|p| p.get_number(PROP_WINDOW_WIN32_HWND_POINTER))
        .unwrap_or(0) as HWND
}

impl D3D12Renderer {
    /// This renderer, as a claimed window's `renderer`.
    fn window_owner(&self) -> usize {
        self.device.as_ptr() as usize
    }

    /// Translation of `D3D12_SupportsSwapchainComposition()`.
    pub(super) fn supports_swapchain_composition_internal(
        &self,
        window: Window,
        swapchain_composition: SwapchainComposition,
    ) -> bool {
        let composition = swapchain_composition_index(swapchain_composition);
        let format = SWAPCHAIN_COMPOSITION_TO_TEXTURE_FORMAT[composition];

        let mut format_support = FeatureDataFormatSupport {
            format,
            ..Default::default()
        };
        let res = self
            .device
            .check_feature_support(D3D12_FEATURE_FORMAT_SUPPORT, &mut format_support);
        if res < 0 {
            // Format is apparently unknown
            return false;
        }

        if format_support.support1 & D3D12_FORMAT_SUPPORT1_DISPLAY == 0 {
            return false;
        }

        let Some(entry) = fetch_window_data(window) else {
            let _ = self.set_string_error(
                "Must claim window before querying swapchain composition support!",
            );
            return false;
        };

        // Check the color space support if necessary
        if swapchain_composition != SwapchainComposition::Sdr {
            let window_data = lock(&entry.data);
            let color_space_support = window_data.swapchain.as_ref().map_or(0, |swapchain| {
                swapchain
                    .check_color_space_support(SWAPCHAIN_COMPOSITION_TO_COLOR_SPACE[composition])
            });

            if color_space_support & DXGI_SWAP_CHAIN_COLOR_SPACE_SUPPORT_FLAG_PRESENT == 0 {
                return false;
            }
        }

        true
    }

    /// Translation of `D3D12_SupportsPresentMode()`.
    pub(super) fn supports_present_mode_internal(
        &self,
        _window: Window,
        present_mode: PresentMode,
    ) -> bool {
        match present_mode {
            PresentMode::Immediate | PresentMode::Vsync => true,
            PresentMode::Mailbox => true,
        }
    }

    /// The texture container of a swapchain buffer, with its views.
    /// Translation of `D3D12_INTERNAL_InitializeSwapchainTexture()`.
    ///
    /// Note (upstream): C ignores a failure to get a descriptor and writes
    /// the views to a NULL handle; the error is returned here.
    fn initialize_swapchain_texture(
        &self,
        swapchain: &DxgiSwapChain3,
        composition: SwapchainComposition,
        index: u32,
    ) -> Result<Arc<TextureContainer>> {
        let composition_index = swapchain_composition_index(composition);
        let swapchain_format = SWAPCHAIN_COMPOSITION_TO_TEXTURE_FORMAT[composition_index];

        let swapchain_texture = swapchain
            .buffer(index)
            .map_err(|res| self.set_error("Could not get buffer from swapchain!", res))?;

        let texture_desc = swapchain_texture.desc();
        let info = TextureCreateInfo {
            width: texture_desc.width as u32,
            height: texture_desc.height,
            layer_count_or_depth: 1,
            num_levels: 1,
            texture_type: TextureType::Texture2D,
            usage: TextureUsageFlags::COLOR_TARGET,
            sample_count: SampleCount::One,
            format: SWAPCHAIN_COMPOSITION_TO_SDL_TEXTURE_FORMAT[composition_index],
            props: None,
        };

        // Create the SRV for the swapchain
        let srv_handle =
            self.assign_staging_descriptor_handle(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)?;

        let mut srv_desc = ShaderResourceViewDesc {
            format: swapchain_format,
            shader_4_component_mapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
            view_dimension: D3D12_SRV_DIMENSION_TEXTURE2D,
            ..Default::default()
        };
        srv_desc.u.texture_2d = Tex2dSrv {
            mip_levels: 1,
            most_detailed_mip: 0,
            resource_min_lod_clamp: 0.0,
            plane_slice: 0,
        };

        self.device.create_shader_resource_view(
            &swapchain_texture,
            &srv_desc,
            srv_handle.cpu_handle,
        );

        // Create the RTV for the swapchain
        let rtv_handle = self.assign_staging_descriptor_handle(D3D12_DESCRIPTOR_HEAP_TYPE_RTV)?;

        let mut rtv_desc = RenderTargetViewDesc {
            format: if composition == SwapchainComposition::SdrLinear {
                DXGI_FORMAT_B8G8R8A8_UNORM_SRGB
            } else {
                swapchain_format
            },
            view_dimension: D3D12_RTV_DIMENSION_TEXTURE2D,
            ..Default::default()
        };
        rtv_desc.u.texture_2d = Tex2dRtv {
            mip_slice: 0,
            plane_slice: 0,
        };

        self.device
            .create_render_target_view(&swapchain_texture, &rtv_desc, rtv_handle.cpu_handle);

        let texture = Arc::new(D3D12Texture {
            container: Mutex::new(None),
            subresources: vec![TextureSubresource {
                layer: 0,
                level: 0,
                depth: 1,
                index: 0,
                rtv_handles: vec![rtv_handle],
                uav_handle: None,
                dsv_handle: None,
            }],
            // (kept until the swapchain's textures are released; see the
            // module's documentation)
            resource: swapchain_texture,
            srv_handle: Some(srv_handle),
            reference_count: AtomicI32::new(0),
            info: info.clone(),
        });

        let container = Arc::new(TextureContainer {
            info,
            state: Mutex::new(TextureContainerState {
                active_texture: texture.clone(),
                textures: vec![texture.clone()],
                debug_name: None,
            }),
            can_be_cycled: false,
        });

        *lock(&texture.container) = Some(ContainerRef {
            container: Arc::downgrade(&container),
            index: 0,
        });

        Ok(container)
    }

    /// Resize a window's swapchain buffers to the window, after waiting for
    /// the device. Translation of `D3D12_INTERNAL_ResizeSwapchain()` (the
    /// window's lock is taken after the wait).
    fn resize_swapchain(&self, entry: &WindowEntry) -> Result<()> {
        // Wait so we don't release in-flight views
        let _ = self.wait_internal();

        let mut window_data = lock(&entry.data);
        let window_data = &mut *window_data;

        // Release views and clean up
        window_data.texture_containers.clear();

        let Some(swapchain) = window_data.swapchain.clone() else {
            return Err(self.set_string_error("Could not resize swapchain buffers"));
        };

        // Resize the swapchain
        let res = swapchain.resize_buffers(
            0,                   // Keep buffer count the same
            0,                   // use client window width
            0,                   // use client window height
            DXGI_FORMAT_UNKNOWN, // Keep the old format
            if self.supports_tearing {
                DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING
            } else {
                0
            },
        );
        if res < 0 {
            return Err(self.set_error("Could not resize swapchain buffers", res));
        }

        // Create texture object for the swapchain
        for i in 0..window_data.swapchain_texture_count {
            let container = self.initialize_swapchain_texture(
                &swapchain,
                window_data.swapchain_composition,
                i,
            )?;
            window_data.texture_containers.push(container);
        }

        // (upstream checks the result of ResizeBuffers again here)
        let swapchain_desc = swapchain.desc1();

        window_data.width = swapchain_desc.width;
        window_data.height = swapchain_desc.height;
        window_data.needs_swapchain_recreate = false;
        Ok(())
    }

    /// Release a window's swapchain textures and swapchain. Translation of
    /// `D3D12_INTERNAL_DestroySwapchain()`.
    fn destroy_swapchain(window_data: &mut WindowData) {
        // Release views and clean up
        window_data.texture_containers.clear();

        window_data.swapchain = None;
    }

    /// Make a window's swapchain and its textures. Translation of
    /// `D3D12_INTERNAL_CreateSwapchain()`.
    ///
    /// FIXME (upstream): the check after `GetDesc1()` tests the result of
    /// the window association (`GetDesc1()` returns nothing), so a swapchain
    /// whose window association fails is not made.
    ///
    /// Note (upstream): C fails without an error when the window has no
    /// `HWND`; the error says so here.
    ///
    /// Not translated: precaching the blit pipelines of the swapchain's
    /// format, which upstream makes through its device (the front end's,
    /// which a backend doesn't have here); they are made on the first
    /// blit into the swapchain's format.
    fn create_swapchain(
        &self,
        window_data: &mut WindowData,
        swapchain_composition: SwapchainComposition,
        present_mode: PresentMode,
    ) -> Result<()> {
        // Get the DXGI handle
        let dxgi_handle = window_hwnd(window_data.window);

        let composition_index = swapchain_composition_index(swapchain_composition);
        let swapchain_format = SWAPCHAIN_COMPOSITION_TO_TEXTURE_FORMAT[composition_index];

        // Min swapchain image count is 2
        window_data.swapchain_texture_count =
            sdl_clamp(self.allowed_frames_in_flight.load(Ordering::SeqCst), 2, 3);

        // Initialize the swapchain buffer descriptor
        let swapchain_desc = SwapChainDesc1 {
            width: 0,  // use client window width
            height: 0, // use client window height
            format: swapchain_format,
            sample_desc: crate::render::direct3d11::d3d::SampleDesc {
                count: 1,
                quality: 0,
            },
            buffer_usage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            buffer_count: window_data.swapchain_texture_count,
            scaling: DXGI_SCALING_NONE,
            swap_effect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            alpha_mode: DXGI_ALPHA_MODE_UNSPECIFIED,
            flags: if self.supports_tearing {
                DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING
            } else {
                0
            },
            stereo: 0,
        };

        // Initialize the fullscreen descriptor (if needed)
        let fullscreen_desc = SwapChainFullscreenDesc {
            refresh_rate: DxgiRational {
                numerator: 0,
                denominator: 0,
            },
            scanline_ordering: DXGI_MODE_SCANLINE_ORDER_UNSPECIFIED,
            scaling: DXGI_MODE_SCALING_UNSPECIFIED,
            windowed: 1,
        };

        // SAFETY: IsWindow takes any handle.
        if dxgi_handle.is_null() || unsafe { IsWindow(dxgi_handle) } == 0 {
            return Err(self.set_string_error("Could not create swapchain: no window handle"));
        }

        // Create the swapchain!
        let swapchain = create_swap_chain_for_hwnd(
            &self.factory,
            &self.command_queue,
            dxgi_handle,
            &swapchain_desc,
            &fullscreen_desc,
        )
        .map_err(|res| self.set_error("Could not create swapchain", res))?;

        let swapchain3 = swapchain.query::<IDXGISwapChain3Vtbl>(&IID_IDXGISWAPCHAIN3);
        drop(swapchain);
        let swapchain3 =
            swapchain3.map_err(|res| self.set_error("Could not create IDXGISwapChain3", res))?;

        if swapchain_composition != SwapchainComposition::Sdr {
            // Support already verified if we hit this block
            swapchain3.set_color_space1(SWAPCHAIN_COMPOSITION_TO_COLOR_SPACE[composition_index]);
        }

        /*
         * The swapchain's parent is a separate factory from the factory that
         * we used to create the swapchain, and only that parent can be used to
         * set the window association. Trying to set an association on our factory
         * will silently fail and doesn't even verify arguments or return errors.
         * See https://gamedev.net/forums/topic/634235-dxgidisabling-altenter/4999955/
         */
        let res = match swapchain3.parent_factory() {
            Err(res) => {
                crate::log::warn!(
                    Category::Gpu,
                    "Could not get swapchain parent! Error Code: (0x{:08X})",
                    res as u32
                );
                res
            }
            Ok(parent) => {
                // Disable DXGI window crap
                let res = make_window_association(&parent, dxgi_handle, DXGI_MWA_NO_WINDOW_CHANGES);
                if res < 0 {
                    crate::log::warn!(
                        Category::Gpu,
                        "MakeWindowAssociation failed! Error Code: (0x{:08X})",
                        res as u32
                    );
                }

                // We're done with the parent now
                res
            }
        };

        let swapchain_desc = swapchain3.desc1();
        if res < 0 {
            return Err(self.set_error("Failed to retrieve swapchain descriptor!", res));
        }

        // Initialize the swapchain data
        window_data.present_mode = present_mode;
        window_data.swapchain_composition = swapchain_composition;
        window_data.swapchain_color_space = SWAPCHAIN_COMPOSITION_TO_COLOR_SPACE[composition_index];
        window_data.frame_counter = 0;
        window_data.width = swapchain_desc.width;
        window_data.height = swapchain_desc.height;

        /* If a you are using a FLIP model format you can't create the swapchain as DXGI_FORMAT_B8G8R8A8_UNORM_SRGB.
         * You have to create the swapchain as DXGI_FORMAT_B8G8R8A8_UNORM and then set the render target view's format to DXGI_FORMAT_B8G8R8A8_UNORM_SRGB
         */
        let mut texture_containers =
            Vec::with_capacity(window_data.swapchain_texture_count as usize);
        for i in 0..window_data.swapchain_texture_count {
            texture_containers.push(self.initialize_swapchain_texture(
                &swapchain3,
                swapchain_composition,
                i,
            )?);
        }

        window_data.swapchain = Some(swapchain3);
        window_data.texture_containers = texture_containers;

        Ok(())
    }

    /// Claim a window: make its swapchain. Translation of
    /// `D3D12_ClaimWindow()`.
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
            swapchain: None,
            present_mode: PresentMode::Vsync,
            swapchain_composition: SwapchainComposition::Sdr,
            swapchain_color_space: DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
            frame_counter: 0,
            texture_containers: Vec::new(),
            swapchain_texture_count: 0,
            in_flight_fences: Default::default(),
            width: 0,
            height: 0,
            needs_swapchain_recreate: false,
        };

        self.create_swapchain(
            &mut window_data,
            SwapchainComposition::Sdr,
            PresentMode::Vsync,
        )?;

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

    /// Unclaim a window: destroy its swapchain. Translation of
    /// `D3D12_ReleaseWindow()`.
    pub(super) fn release_window_internal(&self, window: Window) {
        let Some(entry) = fetch_window_data(window) else {
            return;
        };
        if entry.renderer != self.window_owner() {
            let _ = self.set_string_error("Window not claimed by this device");
            return;
        }
        self.release_window_entry(&entry, window);
    }

    /// The release of a claimed window's entry (`D3D12_ReleaseWindow()`
    /// past the checks).
    fn release_window_entry(&self, entry: &Arc<WindowEntry>, window: Window) {
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

            Self::destroy_swapchain(&mut window_data);
        }

        {
            let mut claimed_windows = lock(&self.claimed_windows);
            if let Some(i) = claimed_windows.iter().position(|e| Arc::ptr_eq(e, entry)) {
                claimed_windows.swap_remove(i);
            }
        }

        if let Ok(properties) = window.properties() {
            properties.remove(WINDOW_PROPERTY_DATA);
        }
        // SDL_RemoveWindowEventWatch()
        lock(&entry.watch).take();
    }

    /// Release every claimed window, most recent first
    /// (`D3D12_DestroyDevice()`).
    ///
    /// Note (upstream): C finds each window's data through the window,
    /// which may be destroyed by now; the claimed windows' entries are
    /// released directly here.
    pub(super) fn release_claimed_windows(&self) {
        let claimed_windows = lock(&self.claimed_windows).clone();
        for entry in claimed_windows.iter().rev() {
            let window = lock(&entry.data).window;
            self.release_window_entry(entry, window);
        }
    }

    /// Translation of `D3D12_SetSwapchainParameters()`.
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

        let changed = {
            let window_data = lock(&entry.data);
            swapchain_composition != window_data.swapchain_composition
                || present_mode != window_data.present_mode
        };
        if changed {
            let _ = self.wait_internal();

            // Recreate the swapchain
            let mut window_data = lock(&entry.data);
            Self::destroy_swapchain(&mut window_data);

            return self.create_swapchain(&mut window_data, swapchain_composition, present_mode);
        }

        Ok(())
    }

    /// Translation of `D3D12_SetAllowedFramesInFlight()`.
    pub(super) fn set_allowed_frames_in_flight_internal(
        &self,
        allowed_frames_in_flight: u32,
    ) -> Result<()> {
        self.wait_internal()?;

        let claimed_windows = lock(&self.claimed_windows).clone();

        // Destroy all swapchains
        for entry in &claimed_windows {
            Self::destroy_swapchain(&mut lock(&entry.data));
        }

        // Set the frames in flight value
        self.allowed_frames_in_flight
            .store(allowed_frames_in_flight, Ordering::SeqCst);

        // Recreate all swapchains
        for entry in &claimed_windows {
            let mut window_data = lock(&entry.data);
            let (composition, present_mode) =
                (window_data.swapchain_composition, window_data.present_mode);
            self.create_swapchain(&mut window_data, composition, present_mode)?;
        }

        Ok(())
    }

    /// Translation of `D3D12_GetSwapchainTextureFormat()`.
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
        window_data
            .texture_containers
            .get(window_data.frame_counter as usize)
            .map(|container| container.info.format)
            .ok_or_else(|| self.set_string_error("Window has no swapchain"))
    }

    /// Block until the window's next swapchain texture can be acquired.
    /// Translation of `D3D12_WaitForSwapchain()`.
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

    /// Acquire the window's next swapchain texture for a command buffer,
    /// waiting for its frame's fence if `block`. Translation of
    /// `D3D12_INTERNAL_AcquireSwapchainTexture()`.
    pub(super) fn acquire_swapchain_texture_internal(
        &self,
        block: bool,
        d3d12_command_buffer: &mut D3D12CommandBuffer,
        window: Window,
    ) -> Result<Option<BackendSwapchainTexture>> {
        let Some(entry) = fetch_window_data(window) else {
            return Err(
                self.set_string_error("Cannot acquire swapchain texture from an unclaimed window!")
            );
        };

        if lock(&entry.data).needs_swapchain_recreate {
            self.resize_swapchain(&entry)?;
        }

        let mut window_data = lock(&entry.data);

        let frame_counter = window_data.frame_counter as usize;
        if let Some(fence) = window_data.in_flight_fences[frame_counter].clone() {
            if block {
                // In VSYNC mode, block until the least recent presented frame is done
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

            // (unless another thread remade the swapchain while the lock
            // was let go for the wait)
            if window_data.in_flight_fences[frame_counter]
                .as_ref()
                .is_some_and(|f| Arc::ptr_eq(f, &fence))
            {
                self.release_fence_internal(&fence);

                window_data.in_flight_fences[frame_counter] = None;
            }
        }

        let Some(swapchain) = window_data.swapchain.clone() else {
            return Err(self.set_string_error("Could not acquire swapchain!"));
        };
        let swapchain_index = swapchain.current_back_buffer_index();

        // (the texture holds the buffer: see the module's documentation)
        let Some(container) = window_data
            .texture_containers
            .get(swapchain_index as usize)
            .cloned()
        else {
            return Err(self.set_string_error("Could not acquire swapchain!"));
        };

        // Set up presentation
        d3d12_command_buffer.present_datas.push(PresentData {
            window_data: entry.clone(),
            swapchain_image_index: swapchain_index,
        });

        // Set up resource barrier
        let resource = lock(&container.state).active_texture.clone();
        let barrier_desc = ResourceBarrier {
            ty: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            u: ResourceBarrierUnion {
                transition: ResourceTransitionBarrier {
                    resource: resource.resource.as_ptr().cast(),
                    subresource: 0,
                    state_before: D3D12_RESOURCE_STATE_PRESENT,
                    state_after: D3D12_RESOURCE_STATE_RENDER_TARGET,
                },
            },
        };

        d3d12_command_buffer
            .graphics_command_list
            .resource_barrier(&[barrier_desc]);

        Ok(Some(BackendSwapchainTexture {
            info: container.info.clone(),
            raw: BackendObject(container),
            width: window_data.width,
            height: window_data.height,
        }))
    }
}

/// Flag a window's swapchain for recreation at its new size. Translation of
/// `D3D12_INTERNAL_OnWindowResize()`.
fn watch_window_resize(window: Window, entry: Weak<WindowEntry>) -> EventWatch {
    let window_id = window.id();
    add_window_event_watch(WindowEventWatchPriority::Normal, move |e| {
        let Event::Window(w) = e else {
            return;
        };
        if w.event_type == EventType::WINDOW_PIXEL_SIZE_CHANGED && w.window_id == window_id {
            if let Some(entry) = entry.upgrade() {
                lock(&entry.data).needs_swapchain_recreate = true;
            }
        }
    })
}
