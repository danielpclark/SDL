// Rust translation of include/SDL3/SDL_openxr.h and the OpenXR entry points
// of src/gpu/SDL_gpu.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! OpenXR handles for GPU devices (SDL_openxr.h). For the most part OpenXR
//! operates independently of SDL, but the graphics initialization depends
//! on direct support from the GPU API: a device made with
//! [`PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN`](super::PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN)
//! creates the OpenXR instance and finds the system
//! ([`Device::xr_instance`], [`Device::xr_system_id`]), and makes the
//! session ([`Device::create_xr_session`]) and its swapchains
//! ([`Device::create_xr_swapchain`]), whose images are GPU textures. The
//! application calls the rest of OpenXR itself, through the loader's
//! `xrGetInstanceProcAddr` ([`openxr_get_xr_get_instance_proc_addr`]).
//!
//! The OpenXR loader (Khronos's, which finds the runtime) is loaded at run
//! time, as upstream loads it ([`openxr_load_library`]; see
//! [`hints::OPENXR_LIBRARY`](crate::hints::OPENXR_LIBRARY)): it is the
//! application's to ship on Windows (`openxr_loader.dll`) and Android (the
//! Gradle package `org.khronos.openxr:openxr_loader_for_android`), a system
//! package elsewhere (`libopenxr-loader1` on Debian and Ubuntu).
//!
//! The handles are `u64`s, as SDL_openxr.h declares them when the OpenXR
//! headers aren't included (`XrInstance`, `XrSystemId`, `XrSession`,
//! `XrSwapchain`). The device's instance belongs to the application, which
//! destroys it after the device (`xrDestroyInstance`): the device never
//! does, and keeps its reference to the loader (upstream's
//! `SDL_OpenXR_LoadLibrary()` from the device's creation is never undone),
//! so that the application's OpenXR functions stay valid.

pub(crate) mod gpu_openxr;
pub(crate) mod openxrdyn;
pub(crate) mod sys;
#[cfg(test)]
mod tests;

use std::ffi::c_void;
use std::fmt;
use std::sync::Arc;

use super::{DeviceShared, Owner, Texture, TextureFormat};
use crate::error::{Error, Result};

/// The result of an OpenXR call (`XrResult`): 0 is success, positive
/// values are qualified successes, negative values errors.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct XrResult(pub i32);

impl XrResult {
    /// `XR_SUCCESS`.
    pub const SUCCESS: XrResult = XrResult(0);
    /// `XR_ERROR_VALIDATION_FAILURE`.
    pub const ERROR_VALIDATION_FAILURE: XrResult = XrResult(-1);
    /// `XR_ERROR_RUNTIME_FAILURE`.
    pub const ERROR_RUNTIME_FAILURE: XrResult = XrResult(-2);
    /// `XR_ERROR_INITIALIZATION_FAILED`.
    pub const ERROR_INITIALIZATION_FAILED: XrResult = XrResult(-6);
    /// `XR_ERROR_FUNCTION_UNSUPPORTED`.
    pub const ERROR_FUNCTION_UNSUPPORTED: XrResult = XrResult(-7);
    /// `XR_ERROR_EXTENSION_NOT_PRESENT`.
    pub const ERROR_EXTENSION_NOT_PRESENT: XrResult = XrResult(-9);
    /// `XR_ERROR_HANDLE_INVALID`.
    pub const ERROR_HANDLE_INVALID: XrResult = XrResult(-12);
    /// `XR_ERROR_FORM_FACTOR_UNSUPPORTED`.
    pub const ERROR_FORM_FACTOR_UNSUPPORTED: XrResult = XrResult(-34);
    /// `XR_ERROR_FORM_FACTOR_UNAVAILABLE`.
    pub const ERROR_FORM_FACTOR_UNAVAILABLE: XrResult = XrResult(-35);

    /// `XR_SUCCEEDED(result)`.
    pub const fn succeeded(self) -> bool {
        self.0 >= 0
    }

    /// `XR_FAILED(result)`.
    pub const fn failed(self) -> bool {
        self.0 < 0
    }
}

impl fmt::Display for XrResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "XrResult {}", self.0)
    }
}

/// A failed OpenXR entry point: its `XrResult`, and SDL's error when it
/// sets one (what `SDL_GetError()` would give; upstream sets none when the
/// runtime's call fails).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct XrError {
    result: XrResult,
    error: Option<Error>,
}

impl XrError {
    /// The result and SDL's error.
    pub(crate) fn new(result: XrResult, error: Error) -> XrError {
        XrError {
            result,
            error: Some(error),
        }
    }

    /// A runtime's result, without an error of SDL's.
    #[allow(dead_code)] // (for the backends)
    pub(crate) fn from_result(result: XrResult) -> XrError {
        XrError {
            result,
            error: None,
        }
    }

    /// The `XrResult` the entry point returned.
    pub fn result(&self) -> XrResult {
        self.result
    }

    /// The error SDL set, if any.
    pub fn error(&self) -> Option<&Error> {
        self.error.as_ref()
    }
}

impl fmt::Display for XrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.error {
            Some(error) => write!(f, "{error} ({})", self.result),
            None => write!(f, "OpenXR call failed ({})", self.result),
        }
    }
}

impl std::error::Error for XrError {}

gpu_flags! {
    /// How a swapchain's images are used (`XrSwapchainUsageFlags`).
    XrSwapchainUsageFlags(u64) {
        /// `XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT`.
        COLOR_ATTACHMENT = sys::XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT;
        /// `XR_SWAPCHAIN_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT`.
        DEPTH_STENCIL_ATTACHMENT = sys::XR_SWAPCHAIN_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT;
        /// `XR_SWAPCHAIN_USAGE_UNORDERED_ACCESS_BIT`.
        UNORDERED_ACCESS = sys::XR_SWAPCHAIN_USAGE_UNORDERED_ACCESS_BIT;
        /// `XR_SWAPCHAIN_USAGE_TRANSFER_SRC_BIT`.
        TRANSFER_SRC = sys::XR_SWAPCHAIN_USAGE_TRANSFER_SRC_BIT;
        /// `XR_SWAPCHAIN_USAGE_TRANSFER_DST_BIT`.
        TRANSFER_DST = sys::XR_SWAPCHAIN_USAGE_TRANSFER_DST_BIT;
        /// `XR_SWAPCHAIN_USAGE_SAMPLED_BIT`.
        SAMPLED = sys::XR_SWAPCHAIN_USAGE_SAMPLED_BIT;
        /// `XR_SWAPCHAIN_USAGE_MUTABLE_FORMAT_BIT`.
        MUTABLE_FORMAT = sys::XR_SWAPCHAIN_USAGE_MUTABLE_FORMAT_BIT;
    }
}

/// The create info of an OpenXR session (`XrSessionCreateInfo`), "sans the
/// system ID": the device gives its own, and prepends its graphics binding
/// to the chain of structures.
#[derive(Clone, Copy, Debug, Default)]
pub struct XrSessionCreateInfo {
    /// The application's chain of OpenXR structures (`next`), after the
    /// device's graphics binding.
    pub next: XrNext,
    /// `createFlags` (`XrSessionCreateFlags`, none defined by OpenXR 1.1).
    pub create_flags: u64,
}

impl XrSessionCreateInfo {
    /// The C structure, without the system ID.
    pub(crate) fn to_sys(self) -> sys::XrSessionCreateInfo {
        sys::XrSessionCreateInfo {
            ty: sys::XR_TYPE_SESSION_CREATE_INFO,
            next: self.next.0,
            create_flags: self.create_flags,
            system_id: sys::XR_NULL_SYSTEM_ID,
        }
    }
}

/// The create info of an OpenXR swapchain (`XrSwapchainCreateInfo`), "sans
/// the format": the format is a GPU texture format, which the backend
/// gives the runtime as its own.
#[derive(Clone, Copy, Debug, Default)]
pub struct XrSwapchainCreateInfo {
    /// The application's chain of OpenXR structures (`next`).
    pub next: XrNext,
    /// `createFlags` (`XrSwapchainCreateFlags`).
    pub create_flags: u64,
    /// `usageFlags`.
    pub usage_flags: XrSwapchainUsageFlags,
    /// `sampleCount`.
    pub sample_count: u32,
    /// `width`.
    pub width: u32,
    /// `height`.
    pub height: u32,
    /// `faceCount`.
    pub face_count: u32,
    /// `arraySize`.
    pub array_size: u32,
    /// `mipCount`.
    pub mip_count: u32,
}

impl XrSwapchainCreateInfo {
    /// The C structure, without the format.
    pub(crate) fn to_sys(self) -> sys::XrSwapchainCreateInfo {
        sys::XrSwapchainCreateInfo {
            ty: sys::XR_TYPE_SWAPCHAIN_CREATE_INFO,
            next: self.next.0,
            create_flags: self.create_flags,
            usage_flags: self.usage_flags.bits(),
            format: 0,
            sample_count: self.sample_count,
            width: self.width,
            height: self.height,
            face_count: self.face_count,
            array_size: self.array_size,
            mip_count: self.mip_count,
        }
    }
}

/// A create info's chain of OpenXR structures (its `next` pointer): none
/// by default ([`XrNext::NONE`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XrNext(*const c_void);

impl XrNext {
    /// No structures (`NULL`).
    pub const NONE: XrNext = XrNext(std::ptr::null());

    /// A chain of OpenXR structures.
    ///
    /// # Safety
    ///
    /// `next` is null or a chain of OpenXR structures that the call the
    /// create info is given to (`xrCreateSession`, `xrCreateSwapchain`)
    /// takes, valid until that call returns.
    pub const unsafe fn new(next: *const c_void) -> XrNext {
        XrNext(next)
    }

    /// The pointer.
    pub const fn as_ptr(self) -> *const c_void {
        self.0
    }
}

impl Default for XrNext {
    fn default() -> XrNext {
        XrNext::NONE
    }
}

/// The OpenXR instance and system of a device made with OpenXR (what
/// upstream's backends write to the `SDL_PROP_GPU_DEVICE_CREATE_XR_*_POINTER`
/// out pointers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct XrDeviceHandles {
    pub(crate) instance: u64,
    pub(crate) system_id: u64,
}

/// An OpenXR swapchain and its images, GPU textures indexed by
/// `xrAcquireSwapchainImage`'s index (the `XrSwapchain` and the texture
/// array of `SDL_CreateGPUXRSwapchain()`).
///
/// [`XrSwapchain::destroy`] destroys it (`SDL_DestroyGPUXRSwapchain()`);
/// dropping it does too, without the result. The textures belong to the
/// swapchain: they aren't released by the front end.
pub struct XrSwapchain {
    handle: u64,
    textures: Vec<Texture>,
    /// `None` once destroyed.
    device: Option<Arc<DeviceShared>>,
}

impl fmt::Debug for XrSwapchain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("XrSwapchain")
            .field("handle", &self.handle)
            .field("textures", &self.textures.len())
            .finish_non_exhaustive()
    }
}

impl XrSwapchain {
    /// The `XrSwapchain` handle, for the application's OpenXR calls
    /// (`xrAcquireSwapchainImage`...).
    pub fn handle(&self) -> u64 {
        self.handle
    }

    /// The swapchain's images, as many as `xrEnumerateSwapchainImages`
    /// gives, indexed by `xrAcquireSwapchainImage`'s index.
    pub fn textures(&self) -> &[Texture] {
        &self.textures
    }

    /// Destroy the swapchain and its textures. Translation of
    /// `SDL_DestroyGPUXRSwapchain()`.
    ///
    /// Note (upstream): its "Invalid GPU Texture handle." for an array
    /// that isn't the swapchain's (whose textures it frees up to that one)
    /// can't happen here: the swapchain holds its own textures.
    pub fn destroy(mut self) -> std::result::Result<(), XrError> {
        self.destroy_now()
    }

    fn destroy_now(&mut self) -> std::result::Result<(), XrError> {
        let Some(device) = self.device.take() else {
            return Ok(());
        };
        let raws: Vec<_> = self.textures.iter().map(|t| t.raw.clone()).collect();
        let result = device.driver.destroy_xr_swapchain(self.handle, &raws);
        drop(raws);
        self.textures.clear();
        result
    }
}

impl Drop for XrSwapchain {
    fn drop(&mut self) {
        let _ = self.destroy_now();
    }
}

impl super::Device {
    /// The OpenXR instance of a device made with OpenXR (what upstream
    /// writes to `SDL_PROP_GPU_DEVICE_CREATE_XR_INSTANCE_POINTER`'s out
    /// pointer); `None` for a device made without it. The application
    /// destroys it (`xrDestroyInstance`) after the device.
    ///
    /// Note (upstream): upstream's backends refuse a device made with OpenXR
    /// without its two out pointers ("You must specify an out pointer for
    /// the OpenXR instance"...); here the handles are the device's, so the
    /// check can't fail.
    pub fn xr_instance(&self) -> Option<u64> {
        self.shared.xr.map(|xr| xr.instance)
    }

    /// The OpenXR system of a device made with OpenXR (what upstream writes
    /// to `SDL_PROP_GPU_DEVICE_CREATE_XR_SYSTEM_ID_POINTER`'s out pointer);
    /// `None` for a device made without it.
    pub fn xr_system_id(&self) -> Option<u64> {
        self.shared.xr.map(|xr| xr.system_id)
    }

    /// Create an OpenXR session for the device, with its system and its
    /// graphics binding. Translation of `SDL_CreateGPUXRSession()`.
    pub fn create_xr_session(
        &self,
        createinfo: &XrSessionCreateInfo,
    ) -> std::result::Result<u64, XrError> {
        self.shared.driver.create_xr_session(&createinfo.to_sys())
    }

    /// The GPU texture formats of the session's swapchain formats, in the
    /// runtime's order (those the backend has a texture format for).
    /// Translation of `SDL_GetGPUXRSwapchainFormats()`.
    ///
    /// Upstream's array is terminated by `SDL_GPU_TEXTUREFORMAT_INVALID`
    /// and has its length in `*num_formats`; this is a `Vec`.
    pub fn xr_swapchain_formats(&self, session: u64) -> Result<Vec<TextureFormat>> {
        self.shared.driver.get_xr_swapchain_formats(session)
    }

    /// Create an OpenXR swapchain of the session, in a format of
    /// [`Device::xr_swapchain_formats`](super::Device::xr_swapchain_formats), with its images as GPU textures.
    /// Translation of `SDL_CreateGPUXRSwapchain()`.
    pub fn create_xr_swapchain(
        &self,
        session: u64,
        createinfo: &XrSwapchainCreateInfo,
        format: TextureFormat,
    ) -> std::result::Result<XrSwapchain, XrError> {
        let (handle, images) =
            self.shared
                .driver
                .create_xr_swapchain(session, &createinfo.to_sys(), format)?;
        Ok(XrSwapchain {
            handle,
            textures: images
                .into_iter()
                .map(|(raw, info)| Texture {
                    raw,
                    info,
                    owner: Owner::XrSwapchain(self.shared.clone()),
                })
                .collect(),
            device: Some(self.shared.clone()),
        })
    }
}

/// Load the OpenXR loader: [`hints::OPENXR_LIBRARY`](crate::hints::OPENXR_LIBRARY)'s
/// path, or else the platform's name (`openxr_loader.dll`,
/// `libopenxr_loader.dylib`, `libopenxr_loader.so` on Android,
/// `libopenxr_loader.so.1`). Calls are reference counted; creating a device
/// with OpenXR loads it too. Translation of `SDL_OpenXR_LoadLibrary()`.
///
/// Note (upstream): a library that doesn't export the loader's four
/// global functions (`xrGetInstanceProcAddr`...) is loaded all the same.
pub fn openxr_load_library() -> Result<()> {
    openxrdyn::load_library()
}

/// Release a reference to the OpenXR loader of [`openxr_load_library`],
/// unloading it with the last one (on Android it stays loaded: its runtime
/// can't be initialized again). Translation of `SDL_OpenXR_UnloadLibrary()`.
pub fn openxr_unload_library() {
    openxrdyn::unload_library()
}

/// The address of the loader's `xrGetInstanceProcAddr` (a
/// `PFN_xrGetInstanceProcAddr`), after [`openxr_load_library`] or the
/// creation of a device with OpenXR. Translation of
/// `SDL_OpenXR_GetXrGetInstanceProcAddr()`.
pub fn openxr_get_xr_get_instance_proc_addr() -> Result<usize> {
    openxrdyn::get_xr_get_instance_proc_addr().map(|f| f as usize)
}
