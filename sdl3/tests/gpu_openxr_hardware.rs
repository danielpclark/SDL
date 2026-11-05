// The GPU API's OpenXR hardware checks: a real OpenXR runtime on a GPU.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `#[ignore]`d tests (`cargo test -p sdl3 --test gpu_openxr_hardware --
//! --ignored --nocapture`; see "Hardware checks" in
//! docs/HARDWARE_TESTING.md), for a machine whose OpenXR loader's active
//! runtime is a real one on a GPU: Monado's compositor or SteamVR on Linux;
//! on Windows SteamVR (its null driver needs a GPU but no headset), Windows
//! Mixed Reality, Meta's Link or Virtual Desktop. What `gpu_openxr` can't
//! show on lavapipe and the test runtime (one device that is both the
//! runtime's and the application's, the runtime's own LUID and formats):
//!
//! * on Vulkan (both systems) and Direct3D 12 (Windows), a device made with
//!   OpenXR on the runtime's choice of GPU (Vulkan: its physical device;
//!   Direct3D 12: the adapter of `xrGetD3D12GraphicsRequirementsKHR`'s LUID,
//!   whose minimum feature level is printed: upstream makes the device
//!   with 11_0 whatever it is), its instance and system, its name;
//! * the session, the runtime's swapchain formats (in its order: printed,
//!   at least one color format SDL renders to), a two-layer color swapchain
//!   of them and its textures;
//! * the session's states and frames: each eye's layer cleared, a
//!   projection layer submitted, for up to 120 rendered frames or 10 s;
//!   then the exit request, the session's end and every destruction.
//!
//! The session must begin, its frames go through, and it must end; a
//! runtime that never makes it visible (a headset not worn) is reported,
//! with the states it went through. Without the OpenXR loader, a runtime
//! or its system, a check skips (`openxr`); without the backend, as its
//! capability (`vulkan`, `d3d12`). The session's part is
//! `openxr_support/session.rs`, which `android_openxr_loader`'s device
//! check on a headset runs too (an app, whose main has no test harness).
//! Elsewhere than Linux and Windows there is nothing here.

#![cfg(any(target_os = "linux", windows))]

#[path = "openxr_support/mod.rs"]
mod openxr_support;

#[cfg(windows)]
use openxr_support::xr;
use openxr_support::{loadable, skip};
use sdl3::gpu::{self, Device};
use sdl3::init::{self, InitFlags};
use sdl3::properties::Properties;

/// The OpenXR loader's library.
#[cfg(target_os = "linux")]
const LOADER: &std::ffi::CStr = c"libopenxr_loader.so.1";
#[cfg(windows)]
const LOADER: &std::ffi::CStr = c"openxr_loader.dll";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Backend {
    Vulkan,
    #[cfg(windows)]
    D3D12,
}

/// The device's creation properties: OpenXR, the backend's shader formats.
fn props(backend: Backend) -> Properties {
    let props = Properties::new();
    match backend {
        Backend::Vulkan => props
            .set(gpu::PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN, true)
            .unwrap(),
        #[cfg(windows)]
        Backend::D3D12 => {
            props
                .set(gpu::PROP_GPU_DEVICE_CREATE_SHADERS_DXBC_BOOLEAN, true)
                .unwrap();
            props
                .set(gpu::PROP_GPU_DEVICE_CREATE_SHADERS_DXIL_BOOLEAN, true)
                .unwrap();
        }
    }
    props
        .set(gpu::PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN, false)
        .unwrap();
    props
        .set(gpu::PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN, true)
        .unwrap();
    props
        .set(
            gpu::PROP_GPU_DEVICE_CREATE_XR_APPLICATION_NAME_STRING,
            "gpu_openxr_hardware",
        )
        .unwrap();
    props
}

/// `XrGraphicsRequirementsD3D12KHR`.
#[cfg(windows)]
#[repr(C)]
struct XrGraphicsRequirementsD3D12KHR {
    ty: i32,
    next: *mut std::ffi::c_void,
    adapter_luid: [u32; 2],
    min_feature_level: i32,
}

/// The runtime's Direct3D 12 requirements: its adapter's LUID and minimum
/// feature level.
#[cfg(windows)]
fn d3d12_requirements(gipa: xr::PfnGetInstanceProcAddr, instance: u64, system: u64) -> String {
    type Pfn = unsafe extern "system" fn(u64, u64, *mut XrGraphicsRequirementsD3D12KHR) -> i32;
    let (result, f) = xr::proc_addr(gipa, instance, "xrGetD3D12GraphicsRequirementsKHR");
    let Some(f) = f else {
        return format!("no xrGetD3D12GraphicsRequirementsKHR ({result})");
    };
    // SAFETY: the function of that name.
    let f: Pfn = unsafe { std::mem::transmute(f) };
    let mut requirements = XrGraphicsRequirementsD3D12KHR {
        ty: 1000028002,
        next: std::ptr::null_mut(),
        adapter_luid: [0; 2],
        min_feature_level: 0,
    };
    // SAFETY: the runtime's function, its system and an out-structure.
    let result = unsafe { f(instance, system, &mut requirements) };
    format!(
        "LUID {:08x}:{:08x}, minimum feature level {:#x} ({result})",
        requirements.adapter_luid[1], requirements.adapter_luid[0], requirements.min_feature_level
    )
}

/// The check: a device with OpenXR on the backend, a session, a swapchain
/// and frames on the active runtime.
fn session_check(test: &str, backend: Backend) {
    // (one check at a time: SDL's video and the runtime are the process's)
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if !loadable(LOADER) {
        let _ = skip(test, "openxr", &format!("no OpenXR loader ({LOADER:?})"));
        return;
    }
    init::init(InitFlags::VIDEO).expect("SDL_Init(SDL_INIT_VIDEO)");
    let props = props(backend);
    if !gpu::supports_properties(&props) {
        let _ = skip(
            test,
            match backend {
                Backend::Vulkan => "vulkan",
                #[cfg(windows)]
                Backend::D3D12 => "d3d12",
            },
            "no device with OpenXR on the backend (no runtime, no system, or no backend)",
        );
        init::quit();
        return;
    }
    let device = Device::with_properties(&props).expect("a device with OpenXR");
    #[cfg(windows)]
    if backend == Backend::D3D12 {
        let instance = device.xr_instance().expect("the device's instance");
        let system = device.xr_system_id().expect("the device's system");
        let gipa = gpu::openxr_get_xr_get_instance_proc_addr().expect("xrGetInstanceProcAddr");
        // SAFETY: the loader's xrGetInstanceProcAddr.
        let gipa_fn: xr::PfnGetInstanceProcAddr = unsafe { std::mem::transmute(gipa) };
        println!(
            "{test}: the runtime's Direct3D 12 requirements: {}",
            d3d12_requirements(gipa_fn, instance, system)
        );
    }
    openxr_support::session::run(test, device);
    init::quit();
}

#[test]
#[ignore = "needs a real OpenXR runtime on a GPU (see docs/HARDWARE_TESTING.md)"]
fn hardware_vulkan_openxr_session_on_the_active_runtime() {
    session_check(
        "hardware_vulkan_openxr_session_on_the_active_runtime",
        Backend::Vulkan,
    );
}

#[cfg(windows)]
#[test]
#[ignore = "needs a real OpenXR runtime on a GPU (see docs/HARDWARE_TESTING.md)"]
fn hardware_d3d12_openxr_session_on_the_active_runtime() {
    session_check(
        "hardware_d3d12_openxr_session_on_the_active_runtime",
        Backend::D3D12,
    );
}
