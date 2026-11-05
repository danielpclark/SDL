// Tests of the GPU API's OpenXR functions on Android with the Khronos
// OpenXR loader for Android, and the device check on a headset's runtime.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, whose package
//! holds the Khronos OpenXR loader for Android (`libopenxr_loader.so`, of
//! the Maven package `org.khronos.openxr:openxr_loader_for_android`, the
//! loader upstream names there; the runner's `libs=`, found in
//! `SDL3_ANDROID_LIBRARY_PATH`), whose main function traces, against
//! upstream's C SDL, what an app sees with the loader and no runtime (the
//! emulator has no runtime broker):
//!
//! * the Vulkan backend says it supports an OpenXR device (its PrepareDriver
//!   doesn't look on Android), and the device fails in CreateDevice: the
//!   loader loaded and initialized (`xrInitializeLoaderKHR` with the app's
//!   Java VM and activity: the loader's own), then the runtime's Vulkan
//!   extension not found (the loader finds no runtime:
//!   `xrEnumerateInstanceExtensionProperties` fails), which upstream logs
//!   (a debug line) without an error (traced as `failed`, the Rust's error
//!   outside the trace: PrepareDriver's "Failed to find a suitable OpenXR
//!   GPU extension." isn't reached on Android);
//! * the library loaded by the program, the loader's global functions
//!   through its `xrGetInstanceProcAddr` (no API layers, the extensions'
//!   query refused with `XR_ERROR_RUNTIME_UNAVAILABLE`), the library
//!   unloaded (Android keeps it, and its `xrGetInstanceProcAddr`).
//!
//! The SDL log lines of the GPU category are in the trace. The expected
//! trace is upstream's C SDL's, from the same program in C (built with
//! upstream's android-project, Gradle and CMake, the loader in its
//! `jniLibs`, run by the runner's `--run-apk`) on the same emulator (API
//! 35, Google APIs, x86-64). Without the loader in the package (`openxr`)
//! it skips, unless `SDL3_TEST_REQUIRE` asks for it.
//!
//! With `SDL3_OPENXR_DEVICE=1` (which reaches the app through its
//! manifest, as the other variables) it is the device check of a headset
//! instead (a Meta Quest, an Android XR device, whose runtime the loader
//! finds through its runtime broker; the runner's `SDL3_ANDROID_XR=1` gives
//! the package the manifest such a runtime asks for): a Vulkan device made
//! with OpenXR on the runtime's choice of GPU, its session, the runtime's
//! swapchain formats, a two-layer color swapchain and up to 120 frames
//! rendered (`openxr_support/session.rs`, as `gpu_openxr_hardware`'s), then
//! every destruction; its exit status says whether everything came (see
//! "Android device" in docs/HARDWARE_TESTING.md). Elsewhere this program
//! does nothing.

#[cfg(not(target_os = "android"))]
fn main() {}

/// The runner's words: run as an app, the Khronos loader in the package.
#[cfg(target_os = "android")]
#[used]
#[link_section = ".sdl3_runner"]
static RUNNER: [u8; 29] = *b"app libs=libopenxr_loader.so\0";

#[cfg(target_os = "android")]
mod android_support;

#[cfg(target_os = "android")]
#[path = "openxr_support/xr.rs"]
mod xr;

#[cfg(target_os = "android")]
#[path = "openxr_support/session.rs"]
mod session;

#[cfg(target_os = "android")]
fn main() -> std::process::ExitCode {
    android_support::start();
    let status = android_support::checked(app::main);
    android_support::finish(status)
}

#[cfg(target_os = "android")]
mod app {
    use std::sync::Mutex;

    use sdl3::gpu::{self, Device};
    use sdl3::init::{self, InitFlags};
    use sdl3::log::{Category, Priority};
    use sdl3::properties::Properties;

    use super::android_support::{check_trace, packaged_library, skip_app};
    use super::xr;

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
init: ok (android)
xrGetInstanceProcAddr: error: The OpenXR loader has not been loaded
supports xr: yes
log 3: Failed to find a compatible OpenXR vulkan extension
device: failed
load library: ok
xrGetInstanceProcAddr: ok
xrEnumerateApiLayerProperties: 0, 0 layers
xrEnumerateInstanceExtensionProperties: -51
library unloaded
xrGetInstanceProcAddr after the unload: ok
quit
";

    /// The loader's library, in the package.
    const LOADER: &str = "libopenxr_loader.so";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// A trace line.
    fn t(line: impl Into<String>) {
        let line = line.into();
        println!("trace: {line}");
        TRACE.lock().unwrap_or_else(|e| e.into_inner()).push(line);
    }

    fn yes(b: bool) -> &'static str {
        if b {
            "yes"
        } else {
            "no"
        }
    }

    fn result<T>(r: &sdl3::Result<T>) -> String {
        match r {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {e}"),
        }
    }

    /// The creation properties of an OpenXR device: SPIR-V, no debug mode,
    /// no verbose logs.
    fn xr_props() -> Properties {
        let props = Properties::new();
        props
            .set(gpu::PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN, true)
            .unwrap();
        props
            .set(gpu::PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN, false)
            .unwrap();
        props
            .set(gpu::PROP_GPU_DEVICE_CREATE_VERBOSE_BOOLEAN, false)
            .unwrap();
        props
            .set(gpu::PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN, true)
            .unwrap();
        props
    }

    pub fn main() {
        if !packaged_library(LOADER) {
            skip_app(
                "openxr",
                "the package has no Khronos OpenXR loader for Android (libopenxr_loader.so, in SDL3_ANDROID_LIBRARY_PATH)",
            );
            return;
        }
        if sdl3::stdlib::getenv_unsafe("SDL3_OPENXR_DEVICE").as_deref() == Some("1") {
            device_check();
            return;
        }
        sdl3::log::set_output(|record| {
            if record.category == Category::Gpu {
                t(format!(
                    "log {}: {}",
                    record.priority as i32, record.message
                ));
            } else {
                println!("log: {}", record.message);
            }
        });
        sdl3::log::set_priority(Category::Gpu, Priority::Verbose);

        match init::init(InitFlags::VIDEO) {
            Ok(()) => t(format!(
                "init: ok ({})",
                sdl3::video::current_video_driver().unwrap_or("?")
            )),
            Err(e) => {
                t(format!("init: error: {e}"));
                return;
            }
        }

        t(format!(
            "xrGetInstanceProcAddr: {}",
            result(&gpu::openxr_get_xr_get_instance_proc_addr())
        ));
        t(format!(
            "supports xr: {}",
            yes(gpu::supports_properties(&xr_props()))
        ));
        match Device::with_properties(&xr_props()) {
            Ok(device) => t(format!("device: ok ({})", device.driver())),
            // (C sets no error there: its SDL_GetError() isn't traced)
            Err(e) => {
                t("device: failed");
                println!("rust: device: {e}");
            }
        }

        t(format!(
            "load library: {}",
            result(&gpu::openxr_load_library())
        ));
        match gpu::openxr_get_xr_get_instance_proc_addr() {
            Ok(gipa) => {
                t("xrGetInstanceProcAddr: ok");
                global_functions(gipa);
            }
            Err(e) => t(format!("xrGetInstanceProcAddr: error: {e}")),
        }
        gpu::openxr_unload_library();
        t("library unloaded");
        t(format!(
            "xrGetInstanceProcAddr after the unload: {}",
            result(&gpu::openxr_get_xr_get_instance_proc_addr())
        ));

        init::quit();
        t("quit");

        let trace = TRACE.lock().unwrap_or_else(|e| e.into_inner()).clone();
        check_trace(&trace, EXPECTED);
    }

    /// `xrEnumerateApiLayerProperties` (as `PFN_xrEnumerateApiLayerProperties`).
    type EnumerateApiLayerProperties = unsafe extern "system" fn(u32, *mut u32, *mut u8) -> i32;
    /// `xrEnumerateInstanceExtensionProperties`.
    type EnumerateInstanceExtensionProperties =
        unsafe extern "system" fn(*const std::ffi::c_char, u32, *mut u32, *mut u8) -> i32;

    /// The loader's global functions through its `xrGetInstanceProcAddr`:
    /// the API layers' and the extensions' counts.
    fn global_functions(gipa: usize) {
        // SAFETY: the loader's xrGetInstanceProcAddr.
        let gipa: xr::PfnGetInstanceProcAddr = unsafe { std::mem::transmute(gipa) };
        let (r, f) = xr::proc_addr(gipa, 0, "xrEnumerateApiLayerProperties");
        match f {
            Some(f) => {
                // SAFETY: the function of that name.
                let f: EnumerateApiLayerProperties = unsafe { std::mem::transmute(f) };
                let mut count = 0;
                // SAFETY: a count query.
                let r = unsafe { f(0, &mut count, std::ptr::null_mut()) };
                t(format!(
                    "xrEnumerateApiLayerProperties: {r}, {count} layers"
                ));
            }
            None => t(format!("xrEnumerateApiLayerProperties: lookup {r}")),
        }
        let (r, f) = xr::proc_addr(gipa, 0, "xrEnumerateInstanceExtensionProperties");
        match f {
            Some(f) => {
                // SAFETY: the function of that name.
                let f: EnumerateInstanceExtensionProperties = unsafe { std::mem::transmute(f) };
                let mut count = 0;
                // SAFETY: a count query.
                let r = unsafe { f(std::ptr::null(), 0, &mut count, std::ptr::null_mut()) };
                t(format!("xrEnumerateInstanceExtensionProperties: {r}"));
            }
            None => t(format!(
                "xrEnumerateInstanceExtensionProperties: lookup {r}"
            )),
        }
    }

    /// The device check on a headset's runtime.
    fn device_check() {
        const TEST: &str = "android_openxr_loader";
        init::init(InitFlags::VIDEO).expect("SDL_Init(SDL_INIT_VIDEO)");
        let props = xr_props();
        props
            .set(gpu::PROP_GPU_DEVICE_CREATE_XR_APPLICATION_NAME_STRING, TEST)
            .unwrap();
        assert!(
            gpu::supports_properties(&props),
            "no Vulkan device with OpenXR"
        );
        let device = Device::with_properties(&props).expect("a device with OpenXR");
        super::session::run(TEST, device);
        init::quit();
        println!("{TEST}: the device check passed");
    }
}
