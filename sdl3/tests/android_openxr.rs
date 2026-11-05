// Tests of the GPU API's OpenXR functions on Android: the loader's
// initialization with the app's Java VM, the instance's Android
// information, and a session on the Vulkan backend, on the repository's
// test runtime.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, whose package
//! holds the repository's test OpenXR runtime (sdl3-openxr-test-runtime,
//! built for the app's ABI beside the program: the runner's `libs=`) and
//! no OpenXR loader, whose main function traces, against upstream's C SDL:
//!
//! * no loader: `openxr_get_xr_get_instance_proc_addr()` and
//!   `openxr_load_library()` fail (upstream's load error); the Vulkan
//!   backend says it supports an OpenXR device all the same (its
//!   PrepareDriver doesn't look on Android: the Quest's runtime can't have
//!   its instance made and destroyed there, then made again), so the
//!   device fails in CreateDevice, with the load error and the debug
//!   build's assertion there ("This should have failed in PrepareDevice
//!   first!", which Android's PrepareDriver can't have done: traced);
//! * the runtime loaded in the loader's place (`SDL_OPENXR_LIBRARY`), its
//!   calls failing as its failure list says (`SDL3_OPENXR_TEST_RUNTIME_FAIL`,
//!   set before it's loaded; the program holds a reference to the library
//!   of its own, so that its calls are counted from its loading): the
//!   lookup of `xrInitializeLoaderKHR` (`XR_KHR_loader_init`, which the
//!   runtime has on Android) failing, then the call itself, each a device
//!   that fails with upstream's message (the library is unloaded after
//!   each, as upstream does); then the library loaded (the loader
//!   initialized with the app's Java VM and activity, which the runtime
//!   checks), unloaded (Android keeps it, and the loader initialized: its
//!   `xrGetInstanceProcAddr` stays), and loaded again with the hint naming
//!   another library (the kept one, not initialized again: the runtime's
//!   third `xrInitializeLoaderKHR` would fail);
//! * a device on the runtime: its instance (with
//!   `XR_KHR_android_create_instance` and the app's Java VM and activity
//!   in its chain, which the runtime checks), system and physical device
//!   (the emulator's SwiftShader, the runtime's choice), the session, the
//!   swapchain formats (the runtime's order), swapchains of one and two
//!   layers; the session's states, and frames rendered into the images (a
//!   clear of each layer) and read back exactly, with a projection layer
//!   of both eyes; the exit, the swapchains destroyed, the session, the
//!   device, the instance (the application's, after the device).
//!
//! Then, outside the trace, the runtime's own checks of what SDL gave it:
//! its `xrInitializeLoaderKHR` called a third time (the failure list's
//! failure: SDL called it twice), then with a Java VM that isn't the
//! process's, a string as the context (both refused), and SDL's VM and
//! activity.
//!
//! The SDL log lines of the GPU category are in the trace. The expected
//! trace is upstream's C SDL's, from the same program in C (built with
//! upstream's android-project, Gradle and CMake, the test runtime in its
//! `jniLibs`, run by the runner's `--run-apk`) on the same emulator (API
//! 35, Google APIs, x86-64): debug builds of both, whose assertions are on
//! (the assertion's lines are left out of the expected trace where they're
//! off). Without the runtime in the package (`openxr-runtime`) it skips,
//! unless `SDL3_TEST_REQUIRE` asks for it. Elsewhere this program does
//! nothing.

#[cfg(not(target_os = "android"))]
fn main() {}

/// The runner's words: run as an app, the test runtime in the package.
#[cfg(target_os = "android")]
#[used]
#[link_section = ".sdl3_runner"]
static RUNNER: [u8; 40] = *b"app libs=libsdl3_openxr_test_runtime.so\0";

#[cfg(target_os = "android")]
mod android_support;

#[cfg(target_os = "android")]
#[path = "openxr_support/xr.rs"]
mod xr;

#[cfg(target_os = "android")]
fn main() -> std::process::ExitCode {
    android_support::start();
    let status = android_support::checked(app::main);
    android_support::finish(status)
}

#[cfg(target_os = "android")]
mod app {
    use std::sync::Mutex;
    use std::time::Duration;

    use sdl3::assert::{AssertData, AssertState};
    use sdl3::gpu::{
        self, ColorTargetInfo, Device, LoadOp, StoreOp, TextureFormat, TextureRegion,
        TextureTransferInfo, TransferBufferCreateInfo, TransferBufferUsage, XrError,
        XrSessionCreateInfo, XrSwapchain, XrSwapchainCreateInfo, XrSwapchainUsageFlags,
    };
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::loadso::SharedObject;
    use sdl3::log::{Category, Priority};
    use sdl3::properties::Properties;
    use sdl3::video::FColor;

    use super::android_support::{check_trace, packaged_library, skip_app};
    use super::xr::{self, Xr};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64, a debug build).
    const EXPECTED: &str = "\
init: ok (android)
xrGetInstanceProcAddr: error: The OpenXR loader has not been loaded
load library: error: Failed to load OpenXR loader library. On Windows, ensure openxr_loader.dll is in your application directory or PATH. On Linux, install the OpenXR loader package (libopenxr-loader) or set LD_LIBRARY_PATH. You can also use the SDL_HINT_OPENXR_LIBRARY hint to specify the loader path.
supports xr: yes
assertion: !\"This should have failed in PrepareDevice first!\"
no loader: error: Failed to load OpenXR loader library. On Windows, ensure openxr_loader.dll is in your application directory or PATH. On Linux, install the OpenXR loader package (libopenxr-loader) or set LD_LIBRARY_PATH. You can also use the SDL_HINT_OPENXR_LIBRARY hint to specify the loader path.
the program's reference: ok
assertion: !\"This should have failed in PrepareDevice first!\"
lookup fails: error: Failed to get xrInitializeLoaderKHR (result: -7)
assertion: !\"This should have failed in PrepareDevice first!\"
initialization fails: error: xrInitializeLoaderKHR failed with result -2
load library: ok
xrGetInstanceProcAddr: ok
library unloaded
xrGetInstanceProcAddr after the unload: ok
load library with another hint: ok
library unloaded
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Android OpenXR loader verified (was initialized by SDL_OpenXR_LoadLibrary)
device: ok (vulkan)
instance: non-null, system: non-null
device name: SwiftShader
session: ok
formats: 52 4 53 12 29 60
swapchain one layer: ok, 3 images
swapchain two layers: ok, 3 images
reference space: 0
log 4: Integrated memory detected, allocating TransferBuffers on device-local memory!
state 1
state 2
begin session: 0
state 3
state 4
state 5
frame 0: one layer 255,128,0,255 255,128,0,255; two layers 255,0,0,255 255,0,0,255, 0,255,0,191 0,255,0,191
frame 1: one layer 255,128,64,255 255,128,64,255; two layers 255,0,64,255 255,0,64,255, 0,255,64,191 0,255,64,191
frame 2: one layer 255,128,128,255 255,128,128,255; two layers 255,0,128,255 255,0,128,255, 0,255,128,191 0,255,128,191
frame 3: one layer 255,128,191,255 255,128,191,255; two layers 255,0,191,255 255,0,191,255, 0,255,191,191 0,255,191,191
request exit: 0
state 4
state 3
state 6
end session: 0
state 1
state 8
session done: true
destroy swapchain one layer: ok
destroy swapchain two layers: ok
destroy space: 0
destroy session: 0
device destroyed
destroy instance: 0
quit
";

    /// The test runtime's library, in the package.
    const RUNTIME: &str = "libsdl3_openxr_test_runtime.so";

    /// The runtime's failures: the first lookup of `xrInitializeLoaderKHR`,
    /// its first call, and its third (which never comes).
    const FAILURES: &str =
        "xrGetInstanceProcAddr:xrInitializeLoaderKHR@1,xrInitializeLoaderKHR@1,xrInitializeLoaderKHR@3";

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

    /// An OpenXR entry point's error as the C program prints it: the
    /// result, and SDL's error when there's one.
    fn xr_error(e: &XrError) -> String {
        match e.error() {
            Some(error) => format!("error {}: {}", e.result().0, error.message()),
            None => format!("error {}", e.result().0),
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

    /// A device that can't be made.
    fn failed_device(what: &str) {
        match Device::with_properties(&xr_props()) {
            Ok(device) => t(format!("{what}: ok ({})", device.driver())),
            Err(e) => t(format!("{what}: error: {e}")),
        }
    }

    pub fn main() {
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
        sdl3::assert::set_assertion_handler(|data: &AssertData| {
            t(format!("assertion: {}", data.condition()));
            AssertState::Ignore
        });
        if !packaged_library(RUNTIME) {
            skip_app(
                "openxr-runtime",
                "the package has no test runtime (sdl3-openxr-test-runtime, built beside the program)",
            );
            return;
        }
        // (the application side's declarations: the layouts of x86-64
        // Linux's C, which Android's are too)
        #[cfg(target_pointer_width = "64")]
        if let Some(difference) = xr::check_layouts() {
            panic!("an OpenXR structure's layout: {difference}");
        }
        // (before the runtime is loaded: it reads the list once)
        std::env::set_var("SDL3_OPENXR_TEST_RUNTIME_FAIL", FAILURES);

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

        no_loader();
        let held = the_runtime_loaded();
        main_scenario();
        runtime_checks();
        drop(held);

        init::quit();
        t("quit");

        let trace = TRACE.lock().unwrap_or_else(|e| e.into_inner()).clone();
        check_trace(&trace, &expected());
    }

    /// The expected trace: without the assertions' lines where they're off
    /// (a release build).
    fn expected() -> String {
        // (SDL_assert()'s level: 2 in a debug build)
        if sdl3::assert::assert_level(cfg!(debug_assertions)) >= 2 {
            return EXPECTED.to_owned();
        }
        EXPECTED
            .lines()
            .filter(|l| !l.starts_with("assertion: "))
            .map(|l| format!("{l}\n"))
            .collect()
    }

    /// No loader in the package (nor a hint).
    fn no_loader() {
        t(format!(
            "xrGetInstanceProcAddr: {}",
            result(&gpu::openxr_get_xr_get_instance_proc_addr())
        ));
        t(format!(
            "load library: {}",
            result(&gpu::openxr_load_library())
        ));
        t(format!(
            "supports xr: {}",
            yes(gpu::supports_properties(&xr_props()))
        ));
        failed_device("no loader");
    }

    /// The runtime in the loader's place: its initialization failing, then
    /// the library loaded, unloaded and loaded again; the program's
    /// reference to it.
    fn the_runtime_loaded() -> Option<SharedObject> {
        hints::set(hints::OPENXR_LIBRARY, RUNTIME).unwrap();
        let held = SharedObject::load(RUNTIME);
        t(format!("the program's reference: {}", result(&held)));
        failed_device("lookup fails");
        failed_device("initialization fails");
        t(format!(
            "load library: {}",
            result(&gpu::openxr_load_library())
        ));
        t(format!(
            "xrGetInstanceProcAddr: {}",
            result(&gpu::openxr_get_xr_get_instance_proc_addr())
        ));
        gpu::openxr_unload_library();
        t("library unloaded");
        t(format!(
            "xrGetInstanceProcAddr after the unload: {}",
            result(&gpu::openxr_get_xr_get_instance_proc_addr())
        ));
        hints::set(hints::OPENXR_LIBRARY, "libsdl3_test_no_such_library.so").unwrap();
        t(format!(
            "load library with another hint: {}",
            result(&gpu::openxr_load_library())
        ));
        gpu::openxr_unload_library();
        t("library unloaded");
        held.ok()
    }

    /// The runtime's own checks of what SDL gave it, not traced (the C
    /// program doesn't make them): its `xrInitializeLoaderKHR` called
    /// again, the third call (the failure list's, so SDL called it twice
    /// and never after Android kept the library), then with a Java VM that
    /// isn't the process's, an object that isn't an `android.content.Context`,
    /// and SDL's VM and activity.
    fn runtime_checks() {
        use std::ffi::c_void;

        use super::android_support::jni;

        /// `XrLoaderInitInfoAndroidKHR`.
        #[repr(C)]
        struct LoaderInitInfoAndroid {
            ty: i32,
            next: *const c_void,
            vm: *mut c_void,
            context: *mut c_void,
        }
        type Initialize = unsafe extern "system" fn(*const LoaderInitInfoAndroid) -> i32;

        let gipa = gpu::openxr_get_xr_get_instance_proc_addr().expect("xrGetInstanceProcAddr");
        // SAFETY: the runtime's xrGetInstanceProcAddr.
        let gipa: xr::PfnGetInstanceProcAddr = unsafe { std::mem::transmute(gipa) };
        let (r, f) = xr::proc_addr(gipa, 0, "xrInitializeLoaderKHR");
        let f = f.unwrap_or_else(|| panic!("the runtime's xrInitializeLoaderKHR: {r}"));
        // SAFETY: the function of that name.
        let initialize: Initialize = unsafe { std::mem::transmute(f) };
        let env = sdl3::android::jni_env()
            .expect("the JNI environment")
            .as_ptr();
        let vm = jni::java_vm(env).0;
        let activity = sdl3::android::activity().expect("the activity").as_ptr();
        let string = jni::new_string(env, "not a context");
        let mut not_a_vm = 0u8;
        let not_a_vm: *mut c_void = (&mut not_a_vm as *mut u8).cast();
        for (what, vm, context, expected) in [
            ("the third call", vm, activity, -2),
            ("another VM", not_a_vm, activity, -1),
            ("a string as the context", vm, string, -1),
            ("SDL's VM and activity", vm, activity, 0),
        ] {
            let info = LoaderInitInfoAndroid {
                ty: 1000089000, // XR_TYPE_LOADER_INIT_INFO_ANDROID_KHR
                next: std::ptr::null(),
                vm,
                context,
            };
            // SAFETY: the runtime's function, with the structure.
            let r = unsafe { initialize(&info) };
            println!("rust: the runtime's xrInitializeLoaderKHR, {what}: {r}");
            assert_eq!(r, expected, "the runtime's xrInitializeLoaderKHR, {what}");
        }
        jni::delete_local_ref(env, string);
        jni::delete_local_ref(env, activity);
    }

    /// The swapchains the main scenario makes: name, size, layers.
    const SWAPCHAINS: [(&str, u32, u32); 2] = [("one layer", 64, 1), ("two layers", 32, 2)];

    /// The frames traced (once the session is FOCUSED).
    const TRACED_FRAMES: u32 = 4;

    /// A layer's clear color in a traced frame.
    fn clear_color(swapchain: usize, layer: u32, frame: u32) -> FColor {
        let f = frame as f32 / 4.0;
        match (swapchain, layer) {
            (0, _) => FColor::new(1.0, 0.5, f, 1.0),
            (_, 0) => FColor::new(1.0, 0.0, f, 1.0),
            _ => FColor::new(0.0, 1.0, f, 0.75),
        }
    }

    /// A pixel of a readback.
    fn pixel(p: &[u8]) -> String {
        format!("{},{},{},{}", p[0], p[1], p[2], p[3])
    }

    /// The device on the runtime: its creation, the session, the
    /// swapchains, the frames, the destruction.
    fn main_scenario() {
        let device = match Device::with_properties(&xr_props()) {
            Ok(device) => device,
            Err(e) => {
                t(format!("device: error: {e}"));
                return;
            }
        };
        t(format!("device: ok ({})", device.driver()));
        let instance = device.xr_instance().unwrap_or(0);
        let system_id = device.xr_system_id().unwrap_or(0);
        t(format!(
            "instance: {}, system: {}",
            if instance != 0 { "non-null" } else { "null" },
            if system_id != 0 { "non-null" } else { "null" }
        ));
        let name = device
            .properties()
            .get_string(gpu::PROP_GPU_DEVICE_NAME_STRING)
            .unwrap_or_default();
        t(format!(
            "device name: {}",
            name.split_whitespace().next().unwrap_or("")
        ));
        let gipa = match gpu::openxr_get_xr_get_instance_proc_addr() {
            Ok(gipa) => gipa,
            Err(e) => {
                t(format!("xrGetInstanceProcAddr: error: {e}"));
                return;
            }
        };
        let x = Xr::load(gipa, instance);

        let session = match device.create_xr_session(&XrSessionCreateInfo::default()) {
            Ok(session) => {
                t("session: ok");
                session
            }
            Err(e) => {
                t(format!("session: {}", xr_error(&e)));
                return;
            }
        };

        match device.xr_swapchain_formats(session) {
            Ok(formats) => t(format!(
                "formats: {}",
                formats
                    .iter()
                    .map(|f| f.0.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            )),
            Err(e) => t(format!("formats: error: {}", e.message())),
        }

        let mut swapchains: Vec<XrSwapchain> = Vec::new();
        for (name, size, layers) in SWAPCHAINS {
            let createinfo = XrSwapchainCreateInfo {
                usage_flags: XrSwapchainUsageFlags::COLOR_ATTACHMENT
                    | XrSwapchainUsageFlags::TRANSFER_SRC,
                sample_count: 1,
                width: size,
                height: size,
                face_count: 1,
                array_size: layers,
                mip_count: 1,
                ..Default::default()
            };
            match device.create_xr_swapchain(session, &createinfo, TextureFormat::R8G8B8A8_UNORM) {
                Ok(swapchain) => {
                    t(format!(
                        "swapchain {name}: ok, {} images",
                        swapchain.textures().len()
                    ));
                    swapchains.push(swapchain);
                }
                Err(e) => {
                    t(format!("swapchain {name}: {}", xr_error(&e)));
                    return;
                }
            }
        }

        let space_info = xr::XrReferenceSpaceCreateInfo {
            ty: xr::XR_TYPE_REFERENCE_SPACE_CREATE_INFO,
            next: std::ptr::null(),
            reference_space_type: xr::XR_REFERENCE_SPACE_TYPE_LOCAL,
            pose_in_reference_space: xr::XrPosef {
                orientation: [0.0, 0.0, 0.0, 1.0],
                position: [0.0; 3],
            },
        };
        let mut space = 0;
        // SAFETY: the runtime's function, the info and an out-pointer.
        let r = unsafe { (x.xrCreateReferenceSpace)(session, &space_info, &mut space) };
        t(format!("reference space: {r}"));

        let one = 64 * 64 * 4;
        let two = 32 * 32 * 4;
        let mut readback = device
            .create_transfer_buffer(&TransferBufferCreateInfo {
                usage: TransferBufferUsage::Download,
                size: one + 2 * two,
                props: None,
            })
            .expect("the transfer buffer");

        let mut running = false;
        let mut state = 0;
        let mut traced = 0;
        let mut exit_requested = false;
        let mut done = false;
        for _ in 0..3000 {
            if done {
                break;
            }
            loop {
                let mut event = xr::XrEventDataBuffer {
                    ty: xr::XR_TYPE_EVENT_DATA_BUFFER,
                    next: std::ptr::null(),
                    varying: [0; 4000],
                };
                // SAFETY: the runtime's function and a buffer.
                if unsafe { (x.xrPollEvent)(instance, &mut event) } != 0 {
                    break;
                }
                if event.ty != xr::XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED {
                    continue;
                }
                // SAFETY: the buffer holds that event.
                let changed = unsafe {
                    &*(&event as *const xr::XrEventDataBuffer)
                        .cast::<xr::XrEventDataSessionStateChanged>()
                };
                state = changed.state;
                t(format!("state {state}"));
                match state {
                    xr::XR_SESSION_STATE_READY => {
                        let begin = xr::XrSessionBeginInfo {
                            ty: xr::XR_TYPE_SESSION_BEGIN_INFO,
                            next: std::ptr::null(),
                            primary_view_configuration_type:
                                xr::XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO,
                        };
                        // SAFETY: the runtime's function and the info.
                        let r = unsafe { (x.xrBeginSession)(session, &begin) };
                        t(format!("begin session: {r}"));
                        running = r == 0;
                    }
                    xr::XR_SESSION_STATE_STOPPING => {
                        // SAFETY: the runtime's function.
                        let r = unsafe { (x.xrEndSession)(session) };
                        t(format!("end session: {r}"));
                        running = false;
                    }
                    xr::XR_SESSION_STATE_EXITING | xr::XR_SESSION_STATE_LOSS_PENDING => {
                        done = true;
                    }
                    _ => {}
                }
            }
            if !running {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }

            let mut frame_state = xr::XrFrameState {
                ty: xr::XR_TYPE_FRAME_STATE,
                next: std::ptr::null_mut(),
                predicted_display_time: 0,
                predicted_display_period: 0,
                should_render: 0,
            };
            // SAFETY: the runtime's functions, infos and out-structures.
            unsafe {
                let wait = xr::XrHeader::new(xr::XR_TYPE_FRAME_WAIT_INFO);
                let r = (x.xrWaitFrame)(session, &wait, &mut frame_state);
                assert_eq!(r, 0, "xrWaitFrame");
                let begin = xr::XrHeader::new(xr::XR_TYPE_FRAME_BEGIN_INFO);
                let r = (x.xrBeginFrame)(session, &begin);
                assert!(r >= 0, "xrBeginFrame {r}");
            }
            let locate = xr::XrViewLocateInfo {
                ty: xr::XR_TYPE_VIEW_LOCATE_INFO,
                next: std::ptr::null(),
                view_configuration_type: xr::XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO,
                display_time: frame_state.predicted_display_time,
                space,
            };
            let mut view_state = xr::XrViewState {
                ty: xr::XR_TYPE_VIEW_STATE,
                next: std::ptr::null_mut(),
                view_state_flags: 0,
            };
            let mut views = [xr::XrView {
                ty: xr::XR_TYPE_VIEW,
                next: std::ptr::null_mut(),
                pose: xr::XrPosef::default(),
                fov: xr::XrFovf::default(),
            }; 2];
            let mut view_count = 0;
            // SAFETY: the runtime's function, the info and room for 2 views.
            unsafe {
                (x.xrLocateViews)(
                    session,
                    &locate,
                    &mut view_state,
                    2,
                    &mut view_count,
                    views.as_mut_ptr(),
                )
            };

            // (the frames once FOCUSED are traced, their colors by their
            // number; the others are cleared as the first)
            let traced_frame = state == xr::XR_SESSION_STATE_FOCUSED && traced < TRACED_FRAMES;
            let frame = if traced_frame { traced } else { 0 };

            let mut indices = [0u32; 2];
            for (k, index) in indices.iter_mut().enumerate() {
                let acquire = xr::XrHeader::new(xr::XR_TYPE_SWAPCHAIN_IMAGE_ACQUIRE_INFO);
                let wait = xr::XrSwapchainImageWaitInfo {
                    ty: xr::XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO,
                    next: std::ptr::null(),
                    timeout: xr::XR_INFINITE_DURATION,
                };
                // SAFETY: the runtime's functions, infos and an out-pointer.
                unsafe {
                    let handle = swapchains[k].handle();
                    let r = (x.xrAcquireSwapchainImage)(handle, &acquire, index);
                    assert_eq!(r, 0, "xrAcquireSwapchainImage");
                    let r = (x.xrWaitSwapchainImage)(handle, &wait);
                    assert_eq!(r, 0, "xrWaitSwapchainImage");
                }
            }

            let mut cb = device.acquire_command_buffer().expect("a command buffer");
            for (k, &index) in indices.iter().enumerate() {
                let texture = &swapchains[k].textures()[index as usize];
                for layer in 0..SWAPCHAINS[k].2 {
                    let mut target = ColorTargetInfo::new(texture);
                    target.layer_or_depth_plane = layer;
                    target.clear_color = clear_color(k, layer, frame);
                    target.load_op = LoadOp::Clear;
                    target.store_op = StoreOp::Store;
                    match cb.begin_render_pass(&[target], None) {
                        Ok(pass) => pass.end(),
                        Err(e) => t(format!("render pass {k} {layer}: error: {e}")),
                    }
                }
            }
            {
                let mut copy = cb.begin_copy_pass().expect("a copy pass");
                let regions = [(0, 0, 0, 64), (1, 0, one, 32), (1, 1, one + two, 32)];
                for (k, layer, offset, size) in regions {
                    let texture = &swapchains[k].textures()[indices[k] as usize];
                    copy.download_from_texture(
                        &TextureRegion {
                            texture,
                            mip_level: 0,
                            layer,
                            x: 0,
                            y: 0,
                            z: 0,
                            w: size,
                            h: size,
                            d: 1,
                        },
                        &TextureTransferInfo {
                            transfer_buffer: &readback,
                            offset,
                            pixels_per_row: 0,
                            rows_per_layer: 0,
                        },
                    );
                }
                copy.end();
            }
            let fence = cb.submit_and_acquire_fence().expect("the submission");
            device.wait_for_fences(true, &[&fence]).expect("the fence");
            drop(fence);
            if traced_frame {
                let mapped = readback.map(false).expect("the mapping");
                let last = |offset: u32, size: u32| (offset + size - 4) as usize;
                t(format!(
                    "frame {frame}: one layer {} {}; two layers {} {}, {} {}",
                    pixel(&mapped[0..]),
                    pixel(&mapped[last(0, one)..]),
                    pixel(&mapped[one as usize..]),
                    pixel(&mapped[last(one, two)..]),
                    pixel(&mapped[(one + two) as usize..]),
                    pixel(&mapped[last(one + two, two)..]),
                ));
            }

            for swapchain in &swapchains {
                let release = xr::XrHeader::new(xr::XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO);
                // SAFETY: the runtime's function and the info.
                let r = unsafe { (x.xrReleaseSwapchainImage)(swapchain.handle(), &release) };
                assert_eq!(r, 0, "xrReleaseSwapchainImage");
            }

            let projection_views: Vec<xr::XrCompositionLayerProjectionView> = (0..2)
                .map(|eye| xr::XrCompositionLayerProjectionView {
                    ty: xr::XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW,
                    next: std::ptr::null(),
                    pose: views[eye].pose,
                    fov: views[eye].fov,
                    sub_image: xr::XrSwapchainSubImage {
                        swapchain: swapchains[1].handle(),
                        image_rect: [0, 0, 32, 32],
                        image_array_index: eye as u32,
                    },
                })
                .collect();
            let layer = xr::XrCompositionLayerProjection {
                ty: xr::XR_TYPE_COMPOSITION_LAYER_PROJECTION,
                next: std::ptr::null(),
                layer_flags: 0,
                space,
                view_count: 2,
                views: projection_views.as_ptr(),
            };
            let layers =
                [(&layer as *const xr::XrCompositionLayerProjection).cast::<xr::XrHeader>()];
            let end = xr::XrFrameEndInfo {
                ty: xr::XR_TYPE_FRAME_END_INFO,
                next: std::ptr::null(),
                display_time: frame_state.predicted_display_time,
                environment_blend_mode: xr::XR_ENVIRONMENT_BLEND_MODE_OPAQUE,
                layer_count: u32::from(frame_state.should_render != 0),
                layers: layers.as_ptr(),
            };
            // SAFETY: the runtime's function, the info and its layers.
            let r = unsafe { (x.xrEndFrame)(session, &end) };
            assert_eq!(r, 0, "xrEndFrame");

            if traced_frame {
                traced += 1;
                if traced == TRACED_FRAMES && !exit_requested {
                    // SAFETY: the runtime's function.
                    let r = unsafe { (x.xrRequestExitSession)(session) };
                    t(format!("request exit: {r}"));
                    exit_requested = true;
                }
            }
        }
        t(format!("session done: {done}"));

        for (swapchain, (name, ..)) in swapchains.into_iter().zip(SWAPCHAINS) {
            match swapchain.destroy() {
                Ok(()) => t(format!("destroy swapchain {name}: ok")),
                Err(e) => t(format!("destroy swapchain {name}: {}", xr_error(&e))),
            }
        }
        drop(readback);
        // SAFETY: the runtime's functions, with the space and the session.
        unsafe {
            t(format!("destroy space: {}", (x.xrDestroySpace)(space)));
            t(format!(
                "destroy session: {}",
                (x.xrDestroySession)(session)
            ));
        }
        drop(device);
        t("device destroyed");
        // SAFETY: the runtime's function, with the device's instance (the
        // application's to destroy, after the device).
        t(format!("destroy instance: {}", unsafe {
            (x.xrDestroyInstance)(instance)
        }));
    }
}
