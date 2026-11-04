// The OpenXR declarations the test programs' application side uses (the
// session loop: events, frames, swapchain images, a reference space and a
// projection layer), from openxr.h.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Interface facts only (names, values, layouts), declared by hand as the
//! library's `gpu::xr::sys` declares SDL's; the layouts are pinned by
//! [`check_layouts`] to what a C program built against upstream's vendored
//! headers (OpenXR 1.1.63) printed on x86-64 Linux. The functions come
//! from the loader's `xrGetInstanceProcAddr`
//! (`sdl3::gpu::openxr_get_xr_get_instance_proc_addr()`).

#![allow(non_snake_case, dead_code)] // (upstream's names; each program uses some)

use std::ffi::{c_char, c_void, CString};

pub type XrResult = i32;
pub type XrInstance = u64;
pub type XrSession = u64;
pub type XrSwapchain = u64;
pub type XrSpace = u64;
pub type XrTime = i64;
pub type XrDuration = i64;

pub const XR_TYPE_VIEW_LOCATE_INFO: i32 = 6;
pub const XR_TYPE_VIEW: i32 = 7;
pub const XR_TYPE_SESSION_BEGIN_INFO: i32 = 10;
pub const XR_TYPE_VIEW_STATE: i32 = 11;
pub const XR_TYPE_FRAME_END_INFO: i32 = 12;
pub const XR_TYPE_EVENT_DATA_BUFFER: i32 = 16;
pub const XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED: i32 = 18;
pub const XR_TYPE_FRAME_WAIT_INFO: i32 = 33;
pub const XR_TYPE_COMPOSITION_LAYER_PROJECTION: i32 = 35;
pub const XR_TYPE_REFERENCE_SPACE_CREATE_INFO: i32 = 37;
pub const XR_TYPE_FRAME_STATE: i32 = 44;
pub const XR_TYPE_FRAME_BEGIN_INFO: i32 = 46;
pub const XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW: i32 = 48;
pub const XR_TYPE_SWAPCHAIN_IMAGE_ACQUIRE_INFO: i32 = 55;
pub const XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO: i32 = 56;
pub const XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO: i32 = 57;

pub const XR_ENVIRONMENT_BLEND_MODE_OPAQUE: i32 = 1;
pub const XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO: i32 = 2;
pub const XR_REFERENCE_SPACE_TYPE_LOCAL: i32 = 2;
pub const XR_INFINITE_DURATION: XrDuration = 0x7fff_ffff_ffff_ffff;

pub const XR_SESSION_STATE_READY: i32 = 2;
pub const XR_SESSION_STATE_FOCUSED: i32 = 5;
pub const XR_SESSION_STATE_STOPPING: i32 = 6;
pub const XR_SESSION_STATE_LOSS_PENDING: i32 = 7;
pub const XR_SESSION_STATE_EXITING: i32 = 8;

/// A structure with only its type and chain (`XrFrameWaitInfo`,
/// `XrFrameBeginInfo`, `XrSwapchainImageAcquireInfo`,
/// `XrSwapchainImageReleaseInfo`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XrHeader {
    pub ty: i32,
    pub next: *const c_void,
}

impl XrHeader {
    pub fn new(ty: i32) -> XrHeader {
        XrHeader {
            ty,
            next: std::ptr::null(),
        }
    }
}

/// `XrEventDataBuffer`.
#[repr(C)]
pub struct XrEventDataBuffer {
    pub ty: i32,
    pub next: *const c_void,
    pub varying: [u8; 4000],
}

/// `XrEventDataSessionStateChanged`.
#[repr(C)]
pub struct XrEventDataSessionStateChanged {
    pub ty: i32,
    pub next: *const c_void,
    pub session: XrSession,
    pub state: i32,
    pub time: XrTime,
}

/// `XrSessionBeginInfo`.
#[repr(C)]
pub struct XrSessionBeginInfo {
    pub ty: i32,
    pub next: *const c_void,
    pub primary_view_configuration_type: i32,
}

/// `XrFrameState`.
#[repr(C)]
pub struct XrFrameState {
    pub ty: i32,
    pub next: *mut c_void,
    pub predicted_display_time: XrTime,
    pub predicted_display_period: XrDuration,
    pub should_render: u32,
}

/// `XrFrameEndInfo`.
#[repr(C)]
pub struct XrFrameEndInfo {
    pub ty: i32,
    pub next: *const c_void,
    pub display_time: XrTime,
    pub environment_blend_mode: i32,
    pub layer_count: u32,
    pub layers: *const *const XrHeader,
}

/// `XrViewLocateInfo`.
#[repr(C)]
pub struct XrViewLocateInfo {
    pub ty: i32,
    pub next: *const c_void,
    pub view_configuration_type: i32,
    pub display_time: XrTime,
    pub space: XrSpace,
}

/// `XrViewState`.
#[repr(C)]
pub struct XrViewState {
    pub ty: i32,
    pub next: *mut c_void,
    pub view_state_flags: u64,
}

/// `XrPosef` (an `XrQuaternionf` and an `XrVector3f`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XrPosef {
    pub orientation: [f32; 4],
    pub position: [f32; 3],
}

/// `XrFovf`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XrFovf {
    pub angles: [f32; 4],
}

/// `XrView`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XrView {
    pub ty: i32,
    pub next: *mut c_void,
    pub pose: XrPosef,
    pub fov: XrFovf,
}

/// `XrSwapchainImageWaitInfo`.
#[repr(C)]
pub struct XrSwapchainImageWaitInfo {
    pub ty: i32,
    pub next: *const c_void,
    pub timeout: XrDuration,
}

/// `XrReferenceSpaceCreateInfo`.
#[repr(C)]
pub struct XrReferenceSpaceCreateInfo {
    pub ty: i32,
    pub next: *const c_void,
    pub reference_space_type: i32,
    pub pose_in_reference_space: XrPosef,
}

/// `XrSwapchainSubImage` (an `XrRect2Di`'s offset and extent).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XrSwapchainSubImage {
    pub swapchain: XrSwapchain,
    pub image_rect: [i32; 4],
    pub image_array_index: u32,
}

/// `XrCompositionLayerProjectionView`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct XrCompositionLayerProjectionView {
    pub ty: i32,
    pub next: *const c_void,
    pub pose: XrPosef,
    pub fov: XrFovf,
    pub sub_image: XrSwapchainSubImage,
}

/// `XrCompositionLayerProjection`.
#[repr(C)]
pub struct XrCompositionLayerProjection {
    pub ty: i32,
    pub next: *const c_void,
    pub layer_flags: u64,
    pub space: XrSpace,
    pub view_count: u32,
    pub views: *const XrCompositionLayerProjectionView,
}

/// `PFN_xrVoidFunction`.
pub type PfnVoid = unsafe extern "system" fn();
/// `PFN_xrGetInstanceProcAddr`.
pub type PfnGetInstanceProcAddr =
    unsafe extern "system" fn(XrInstance, *const c_char, *mut Option<PfnVoid>) -> XrResult;

/// The instance functions the programs call.
pub struct Xr {
    pub xrPollEvent: unsafe extern "system" fn(XrInstance, *mut XrEventDataBuffer) -> XrResult,
    pub xrBeginSession: unsafe extern "system" fn(XrSession, *const XrSessionBeginInfo) -> XrResult,
    pub xrEndSession: unsafe extern "system" fn(XrSession) -> XrResult,
    pub xrRequestExitSession: unsafe extern "system" fn(XrSession) -> XrResult,
    pub xrDestroySession: unsafe extern "system" fn(XrSession) -> XrResult,
    pub xrWaitFrame:
        unsafe extern "system" fn(XrSession, *const XrHeader, *mut XrFrameState) -> XrResult,
    pub xrBeginFrame: unsafe extern "system" fn(XrSession, *const XrHeader) -> XrResult,
    pub xrEndFrame: unsafe extern "system" fn(XrSession, *const XrFrameEndInfo) -> XrResult,
    pub xrLocateViews: unsafe extern "system" fn(
        XrSession,
        *const XrViewLocateInfo,
        *mut XrViewState,
        u32,
        *mut u32,
        *mut XrView,
    ) -> XrResult,
    pub xrAcquireSwapchainImage:
        unsafe extern "system" fn(XrSwapchain, *const XrHeader, *mut u32) -> XrResult,
    pub xrWaitSwapchainImage:
        unsafe extern "system" fn(XrSwapchain, *const XrSwapchainImageWaitInfo) -> XrResult,
    pub xrReleaseSwapchainImage:
        unsafe extern "system" fn(XrSwapchain, *const XrHeader) -> XrResult,
    pub xrCreateReferenceSpace: unsafe extern "system" fn(
        XrSession,
        *const XrReferenceSpaceCreateInfo,
        *mut XrSpace,
    ) -> XrResult,
    pub xrDestroySpace: unsafe extern "system" fn(XrSpace) -> XrResult,
    pub xrDestroyInstance: unsafe extern "system" fn(XrInstance) -> XrResult,
}

/// `xrGetInstanceProcAddr(instance, name)`: the result, and the function.
pub fn proc_addr(
    gipa: PfnGetInstanceProcAddr,
    instance: XrInstance,
    name: &str,
) -> (XrResult, Option<PfnVoid>) {
    let name = CString::new(name).expect("no NUL");
    let mut f = None;
    // SAFETY: the loader's function, a name and an out-pointer.
    let result = unsafe { gipa(instance, name.as_ptr(), &mut f) };
    (result, f)
}

/// A function pointer as another function pointer type.
///
/// # Safety
///
/// `F` is a function pointer type, the function's.
unsafe fn cast<F: Copy>(f: PfnVoid) -> F {
    const { assert!(std::mem::size_of::<F>() == std::mem::size_of::<PfnVoid>()) };
    // SAFETY: the caller's.
    unsafe { std::mem::transmute_copy(&f) }
}

impl Xr {
    /// The functions of an instance, through the loader's
    /// `xrGetInstanceProcAddr` (its address).
    pub fn load(gipa: usize, instance: XrInstance) -> Xr {
        // SAFETY: the loader's xrGetInstanceProcAddr.
        let gipa: PfnGetInstanceProcAddr = unsafe { std::mem::transmute(gipa) };
        macro_rules! f {
            ($name:ident) => {{
                let (result, f) = proc_addr(gipa, instance, stringify!($name));
                let f = f.unwrap_or_else(|| panic!("{}: {result}", stringify!($name)));
                // SAFETY: the function of the name, of the field's type.
                unsafe { cast(f) }
            }};
        }
        Xr {
            xrPollEvent: f!(xrPollEvent),
            xrBeginSession: f!(xrBeginSession),
            xrEndSession: f!(xrEndSession),
            xrRequestExitSession: f!(xrRequestExitSession),
            xrDestroySession: f!(xrDestroySession),
            xrWaitFrame: f!(xrWaitFrame),
            xrBeginFrame: f!(xrBeginFrame),
            xrEndFrame: f!(xrEndFrame),
            xrLocateViews: f!(xrLocateViews),
            xrAcquireSwapchainImage: f!(xrAcquireSwapchainImage),
            xrWaitSwapchainImage: f!(xrWaitSwapchainImage),
            xrReleaseSwapchainImage: f!(xrReleaseSwapchainImage),
            xrCreateReferenceSpace: f!(xrCreateReferenceSpace),
            xrDestroySpace: f!(xrDestroySpace),
            xrDestroyInstance: f!(xrDestroyInstance),
        }
    }
}

/// The layouts of the structures, as the C harness printed them on x86-64
/// Linux (sizes; offsets of the fields after the header): `None` when
/// they match, else the first difference.
#[cfg(target_pointer_width = "64")]
pub fn check_layouts() -> Option<String> {
    use std::mem::{offset_of, size_of};
    let checks: &[(&str, usize, usize)] = &[
        ("XrEventDataBuffer", size_of::<XrEventDataBuffer>(), 4016),
        (
            "XrEventDataSessionStateChanged",
            size_of::<XrEventDataSessionStateChanged>(),
            40,
        ),
        (
            "XrEventDataSessionStateChanged.state",
            offset_of!(XrEventDataSessionStateChanged, state),
            24,
        ),
        ("XrSessionBeginInfo", size_of::<XrSessionBeginInfo>(), 24),
        ("XrFrameState", size_of::<XrFrameState>(), 40),
        (
            "XrFrameState.shouldRender",
            offset_of!(XrFrameState, should_render),
            32,
        ),
        ("XrFrameEndInfo", size_of::<XrFrameEndInfo>(), 40),
        (
            "XrFrameEndInfo.layerCount",
            offset_of!(XrFrameEndInfo, layer_count),
            28,
        ),
        ("XrViewLocateInfo", size_of::<XrViewLocateInfo>(), 40),
        (
            "XrViewLocateInfo.space",
            offset_of!(XrViewLocateInfo, space),
            32,
        ),
        ("XrViewState", size_of::<XrViewState>(), 24),
        ("XrPosef", size_of::<XrPosef>(), 28),
        ("XrFovf", size_of::<XrFovf>(), 16),
        ("XrView", size_of::<XrView>(), 64),
        ("XrView.fov", offset_of!(XrView, fov), 44),
        ("XrHeader", size_of::<XrHeader>(), 16),
        (
            "XrSwapchainImageWaitInfo",
            size_of::<XrSwapchainImageWaitInfo>(),
            24,
        ),
        (
            "XrReferenceSpaceCreateInfo",
            size_of::<XrReferenceSpaceCreateInfo>(),
            48,
        ),
        (
            "XrReferenceSpaceCreateInfo.poseInReferenceSpace",
            offset_of!(XrReferenceSpaceCreateInfo, pose_in_reference_space),
            20,
        ),
        ("XrSwapchainSubImage", size_of::<XrSwapchainSubImage>(), 32),
        (
            "XrSwapchainSubImage.imageArrayIndex",
            offset_of!(XrSwapchainSubImage, image_array_index),
            24,
        ),
        (
            "XrCompositionLayerProjectionView",
            size_of::<XrCompositionLayerProjectionView>(),
            96,
        ),
        (
            "XrCompositionLayerProjectionView.subImage",
            offset_of!(XrCompositionLayerProjectionView, sub_image),
            64,
        ),
        (
            "XrCompositionLayerProjection",
            size_of::<XrCompositionLayerProjection>(),
            48,
        ),
        (
            "XrCompositionLayerProjection.views",
            offset_of!(XrCompositionLayerProjection, views),
            40,
        ),
    ];
    checks
        .iter()
        .find(|&&(_, rust, c)| rust != c)
        .map(|&(name, rust, c)| format!("{name}: {rust}, C's {c}"))
}
