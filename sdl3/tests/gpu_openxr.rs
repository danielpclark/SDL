// Tests of the GPU API's OpenXR functions on the Vulkan backend, against
// Monado's null compositor and the repository's test runtime.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A program (`harness = false`, see `openxr_support`) that runs its
//! scenarios in child processes of its own, each with a private runtime
//! directory, lavapipe and the offscreen video driver, tracing what the
//! application sees:
//!
//! * `no-xr`: a device made without OpenXR, whose OpenXR entry points
//!   (the session, the swapchain formats, a swapchain) fail with an error
//!   (upstream's dereference `renderer->xr`, NULL there, and crash: fixed,
//!   with upstream's C patched the same way);
//! * `no-runtime`: no OpenXR runtime (`XR_RUNTIME_JSON` names none): the
//!   runtime's Vulkan extension isn't found, no backend takes the device;
//! * `no-service`: Monado's runtime without its service:
//!   `xrCreateInstance` fails;
//! * `main`, on Monado's service (the null compositor on lavapipe):
//!   `supports_properties()` with OpenXR; a HANDHELD form factor (no
//!   system); an unknown extension (no instance); the device in debug mode
//!   (Vulkan's validation layer on) with an extension and an API layer of
//!   the application's (`XR_EXT_debug_utils`, whose functions the instance
//!   then has, and the core validation layer), its instance, system and
//!   physical device (lavapipe's, the runtime's choice); the session, the
//!   swapchain formats (the runtime's order), swapchains of one and two
//!   layers and one without `COLOR_ATTACHMENT`; the session's states, and
//!   frames rendered into the images (a clear of each layer) and read back
//!   exactly (the readback swapchains ask for `TRANSFER_SRC`), with a
//!   projection layer of both eyes; the exit, the swapchains destroyed, the
//!   session, the device, the instance (the application's, after the
//!   device).
//!
//! The SDL log lines of the GPU category are in the trace (as the child's
//! lines); the loader's, the runtime's and the validation layers' messages
//! aren't, but they're counted: the Vulkan validation layer's errors
//! (none, now that the swapchain textures' transition covers their
//! subresources; upstream's covers none, 2 errors an image: fixed, and the
//! C patched the same way), and the OpenXR loader's valid usage messages
//! (2: its refusal of the API layer query of each device's instance, the
//! unknown extension's and the main one's, whose structures upstream
//! leaves without their type, which is kept: so the core validation layer
//! is never enabled for debug mode; the application's list enables it
//! here). The frame period and the LUIDs are
//! never traced, nor the images' indices (how many frames come before the
//! FOCUSED state isn't fixed).
//!
//! Then the same on the repository's test runtime (sdl3-openxr-test-runtime,
//! deterministic, with its failure injection), a second trace:
//!
//! * `rt-main`, through the OpenXR loader (a manifest of the runtime in
//!   `XR_RUNTIME_JSON`): `main` again (the runtime's formats, 3 images a
//!   swapchain, the same states); its images have exactly the usage asked
//!   for, so the swapchain without `COLOR_ATTACHMENT` gets a Vulkan
//!   validation error an image (3), upstream's transition of every image to
//!   the color attachment layout (kept, as upstream's C does it; Monado
//!   adds that usage itself);
//! * `rt-loader`, through the loader, and `rt-hint`, the runtime loaded in
//!   the loader's place (`SDL_OPENXR_LIBRARY`): a device, its session, the
//!   formats, a swapchain and its destruction, the session's, the
//!   device's and the instance's (the program holds a reference to the
//!   library of its own, so that the runtime stays loaded and counts its
//!   calls from the start);
//! * `rt-negotiate`: the loader can't negotiate with the runtime;
//! * the other `rt-` scenarios, the same steps with one of the runtime's
//!   calls failing (`SDL3_OPENXR_TEST_RUNTIME_FAIL`), each call SDL makes,
//!   in PrepareDriver (the first call) and in CreateDevice (the second),
//!   the lookups of the extension's functions too (and the API layers'
//!   query, whose failure changes nothing): where upstream sets no error
//!   (the extension's search, the instance, its functions, its system and
//!   the requirements in CreateDevice; the swapchain formats), the trace
//!   says `failed`.
//!
//! The expected traces are upstream's C SDL's, from the same program in C
//! (upstream built with CMake at ea7a2da, patched for the two fixes above,
//! and with its `VkResult` of the runtime's Vulkan instance and device
//! initialized, which it logs uninitialized when the runtime's call fails,
//! run by this harness with `--run-c PATH`, on the CI runner). Their first
//! lines, the packages' versions, are the preambles: not compared. Without
//! its capabilities each part skips (`openxr`: the loader,
//! `monado-service`, the core validation layer, lavapipe; `openxr-runtime`:
//! the test runtime, which sdl3's dev-dependency on it builds beside this
//! program, the loader, the core validation layer, lavapipe), unless
//! `SDL3_TEST_REQUIRE` asks for them. Elsewhere than Linux it does
//! nothing.

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
#[path = "openxr_support/mod.rs"]
mod openxr_support;

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    app::main()
}

#[cfg(target_os = "linux")]
mod app {
    use std::path::{Path, PathBuf};
    use std::process::ExitCode;
    use std::time::Duration;

    use super::openxr_support::xr::{self, Xr};
    use super::openxr_support::{
        missing_openxr, missing_openxr_runtime, run_child, skip, test_runtime,
        write_test_runtime_manifest, Monado, RuntimeDir, Trace, PREFIX, SCENARIO,
        TEST_RUNTIME_FAIL, XR_VALIDATION_LAYER,
    };
    use sdl3::gpu::{
        self, ColorTargetInfo, Device, LoadOp, StoreOp, TextureFormat, TextureRegion,
        TextureTransferInfo, TransferBufferCreateInfo, TransferBufferUsage, XrError,
        XrSessionCreateInfo, XrSwapchain, XrSwapchainCreateInfo, XrSwapchainUsageFlags,
    };
    use sdl3::init::{self, InitFlags};
    use sdl3::log::{Category, Priority};
    use sdl3::properties::Properties;
    use sdl3::video::FColor;

    const PROGRAM: &str = "gpu_openxr";

    /// Upstream's trace (the C program, patched, by this harness on the CI
    /// runner).
    const EXPECTED: &str = "\
versions: libopenxr-loader1 1.0.20~dfsg1-1build2, mesa-vulkan-drivers 25.2.8-0ubuntu0.24.04.4, monado-service 21.0.0+git2905.e26a272c1~dfsg1-2build2, openxr-layer-corevalidation 1.0.20~dfsg1-1build2, vulkan-validationlayers 1.3.275.0-1
-- no-xr
init: ok (offscreen)
device: ok (vulkan)
session: error -7: The GPU device was not created with OpenXR enabled
formats: error: The GPU device was not created with OpenXR enabled
swapchain: error -7: The GPU device was not created with OpenXR enabled
device destroyed
quit
no-xr: exit 0
no-xr: vulkan validation errors: 0
no-xr: openxr valid usage messages: 0
-- no-runtime
init: ok (offscreen)
supports xr: false
device: error: No supported SDL_GPU backend found!
quit
no-runtime: exit 0
no-runtime: vulkan validation errors: 0
no-runtime: openxr valid usage messages: 0
-- no-service
init: ok (offscreen)
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to create OpenXR instance
supports xr: false
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to create OpenXR instance
device: error: No supported SDL_GPU backend found!
quit
no-service: exit 0
no-service: vulkan validation errors: 0
no-service: openxr valid usage messages: 0
-- main
init: ok (offscreen)
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Validation layers enabled, expect debug level performance!
supports xr: true
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to get OpenXR system
handheld: error: No supported SDL_GPU backend found!
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to create OpenXR instance
log 3: Failed to init OpenXR
unknown extension: failed
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Validation layers enabled, expect debug level performance!
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Validation layers enabled, expect debug level performance!
device: ok (vulkan)
instance: non-null, system: non-null
device name: llvmpipe
xrGetInstanceProcAddr: ok
xrCreateDebugUtilsMessengerEXT: 0
session: ok
formats: 7 29 52 53 4 12 9 30 60 58 59 61 62
swapchain one layer: ok, 3 images
swapchain two layers: ok, 3 images
swapchain no color attachment: ok, 3 images
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
destroy swapchain no color attachment: ok
destroy space: 0
destroy session: 0
device destroyed
destroy instance: 0
quit
main: exit 0
main: vulkan validation errors: 0
main: openxr valid usage messages: 2
";

    /// A scenario's environment variables.
    type Env = &'static [(&'static str, &'static str)];

    /// The scenarios: their names, whether Monado's service runs for them,
    /// and their environment.
    const SCENARIOS: &[(&str, bool, Env)] = &[
        ("no-xr", false, &[]),
        (
            "no-runtime",
            false,
            &[(
                "XR_RUNTIME_JSON",
                "/nonexistent/sdl3-test/openxr_runtime.json",
            )],
        ),
        ("no-service", false, &[]),
        ("main", true, &[]),
    ];

    /// Upstream's trace on the test runtime (the C program, patched, by this
    /// harness on the CI runner).
    const EXPECTED_RUNTIME: &str = "\
versions: libopenxr-loader1 1.0.20~dfsg1-1build2, mesa-vulkan-drivers 25.2.8-0ubuntu0.24.04.4, monado-service 21.0.0+git2905.e26a272c1~dfsg1-2build2, openxr-layer-corevalidation 1.0.20~dfsg1-1build2, vulkan-validationlayers 1.3.275.0-1
-- rt-main
init: ok (offscreen)
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Validation layers enabled, expect debug level performance!
supports xr: true
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to get OpenXR system
handheld: error: No supported SDL_GPU backend found!
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to create OpenXR instance
log 3: Failed to init OpenXR
unknown extension: failed
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Validation layers enabled, expect debug level performance!
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Validation layers enabled, expect debug level performance!
device: ok (vulkan)
instance: non-null, system: non-null
device name: llvmpipe
xrGetInstanceProcAddr: ok
xrCreateDebugUtilsMessengerEXT: 0
session: ok
formats: 52 4 53 12 29 60
swapchain one layer: ok, 3 images
swapchain two layers: ok, 3 images
swapchain no color attachment: ok, 3 images
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
destroy swapchain no color attachment: ok
destroy space: 0
destroy session: 0
device destroyed
destroy instance: 0
quit
rt-main: exit 0
rt-main: vulkan validation errors: 3
rt-main: openxr valid usage messages: 2
-- rt-loader
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
device: ok (vulkan)
session: ok
formats: 52 4 53 12 29 60
swapchain: ok
destroy swapchain: ok
destroy session: 0
device destroyed
destroy instance: 0
library unloaded
quit
rt-loader: exit 0
rt-loader: vulkan validation errors: 0
rt-loader: openxr valid usage messages: 1
-- rt-negotiate
init: ok (offscreen)
load library: ok
device: error: No supported SDL_GPU backend found!
library unloaded
quit
rt-negotiate: exit 0
rt-negotiate: vulkan validation errors: 0
rt-negotiate: openxr valid usage messages: 0
-- rt-hint
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
device: ok (vulkan)
session: ok
formats: 52 4 53 12 29 60
swapchain: ok
destroy swapchain: ok
destroy session: 0
device destroyed
destroy instance: 0
library unloaded
quit
rt-hint: exit 0
rt-hint: vulkan validation errors: 0
rt-hint: openxr valid usage messages: 0
-- rt-no-extension
init: ok (offscreen)
load library: ok
device: error: No supported SDL_GPU backend found!
library unloaded
quit
rt-no-extension: exit 0
rt-no-extension: vulkan validation errors: 0
rt-no-extension: openxr valid usage messages: 0
-- rt-extensions
init: ok (offscreen)
load library: ok
device: error: No supported SDL_GPU backend found!
library unloaded
quit
rt-extensions: exit 0
rt-extensions: vulkan validation errors: 0
rt-extensions: openxr valid usage messages: 0
-- rt-extensions-2
init: ok (offscreen)
load library: ok
device: error: No supported SDL_GPU backend found!
library unloaded
quit
rt-extensions-2: exit 0
rt-extensions-2: vulkan validation errors: 0
rt-extensions-2: openxr valid usage messages: 0
-- rt-extensions-3
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to find a compatible OpenXR vulkan extension
device: failed
library unloaded
quit
rt-extensions-3: exit 0
rt-extensions-3: vulkan validation errors: 0
rt-extensions-3: openxr valid usage messages: 0
-- rt-api-layers
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
device: ok (vulkan)
session: ok
formats: 52 4 53 12 29 60
swapchain: ok
destroy swapchain: ok
destroy session: 0
device destroyed
destroy instance: 0
library unloaded
quit
rt-api-layers: exit 0
rt-api-layers: vulkan validation errors: 0
rt-api-layers: openxr valid usage messages: 0
-- rt-instance
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to create OpenXR instance
device: error: No supported SDL_GPU backend found!
library unloaded
quit
rt-instance: exit 0
rt-instance: vulkan validation errors: 0
rt-instance: openxr valid usage messages: 0
-- rt-instance-2
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to create OpenXR instance
log 3: Failed to init OpenXR
device: failed
library unloaded
quit
rt-instance-2: exit 0
rt-instance-2: vulkan validation errors: 0
rt-instance-2: openxr valid usage messages: 0
-- rt-symbols
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to load needed OpenXR instance symbols
device: error: No supported SDL_GPU backend found!
library unloaded
quit
rt-symbols: exit 0
rt-symbols: vulkan validation errors: 0
rt-symbols: openxr valid usage messages: 0
-- rt-symbols-2
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to load required OpenXR instance symbols
log 3: Failed to init OpenXR
device: failed
library unloaded
quit
rt-symbols-2: exit 0
rt-symbols-2: vulkan validation errors: 0
rt-symbols-2: openxr valid usage messages: 0
-- rt-system
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to get OpenXR system
device: error: No supported SDL_GPU backend found!
library unloaded
quit
rt-system: exit 0
rt-system: vulkan validation errors: 0
rt-system: openxr valid usage messages: 0
-- rt-system-2
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to get OpenXR system
log 3: Failed to init OpenXR
device: failed
library unloaded
quit
rt-system-2: exit 0
rt-system-2: vulkan validation errors: 0
rt-system-2: openxr valid usage messages: 0
-- rt-requirements
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to get vulkan graphics requirements, got OpenXR error -2
device: error: No supported SDL_GPU backend found!
library unloaded
quit
rt-requirements: exit 0
rt-requirements: vulkan validation errors: 0
rt-requirements: openxr valid usage messages: 0
-- rt-requirements-2
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to get vulkan graphics requirements, got OpenXR error -2
log 3: Failed to get OpenXR graphics requirements
device: failed
library unloaded
quit
rt-requirements-2: exit 0
rt-requirements-2: vulkan validation errors: 0
rt-requirements-2: openxr valid usage messages: 0
-- rt-requirements-lookup
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to get xrGetVulkanGraphicsRequirements2KHR
device: error: No supported SDL_GPU backend found!
library unloaded
quit
rt-requirements-lookup: exit 0
rt-requirements-lookup: vulkan validation errors: 0
rt-requirements-lookup: openxr valid usage messages: 0
-- rt-vulkan-instance
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to create vulkan instance, reason -2, 0
log 5: Vulkan: Could not create Vulkan instance
device: error: No supported SDL_GPU backend found!
library unloaded
quit
rt-vulkan-instance: exit 0
rt-vulkan-instance: vulkan validation errors: 0
rt-vulkan-instance: openxr valid usage messages: 0
-- rt-vulkan-instance-2
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to create vulkan instance, reason -2, 0
log 5: Vulkan: Could not create Vulkan instance
device: error: Failed to initialize Vulkan!
library unloaded
quit
rt-vulkan-instance-2: exit 0
rt-vulkan-instance-2: vulkan validation errors: 0
rt-vulkan-instance-2: openxr valid usage messages: 0
-- rt-vulkan-instance-vk
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 5: Vulkan: Could not create Vulkan instance
device: error: No supported SDL_GPU backend found!
library unloaded
quit
rt-vulkan-instance-vk: exit 0
rt-vulkan-instance-vk: vulkan validation errors: 0
rt-vulkan-instance-vk: openxr valid usage messages: 0
-- rt-vulkan-instance-lookup
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to get xrCreateVulkanInstanceKHR
log 5: Vulkan: Could not create Vulkan instance
device: error: No supported SDL_GPU backend found!
library unloaded
quit
rt-vulkan-instance-lookup: exit 0
rt-vulkan-instance-lookup: vulkan validation errors: 0
rt-vulkan-instance-lookup: openxr valid usage messages: 0
-- rt-graphics-device
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 6: xrGetVulkanGraphicsDevice2KHR failed, result: -2
log 5: Vulkan: Failed to determine a suitable physical device
device: error: No supported SDL_GPU backend found!
library unloaded
quit
rt-graphics-device: exit 0
rt-graphics-device: vulkan validation errors: 0
rt-graphics-device: openxr valid usage messages: 0
-- rt-graphics-device-2
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
log 6: xrGetVulkanGraphicsDevice2KHR failed, result: -2
log 5: Vulkan: Failed to determine a suitable physical device
device: error: Failed to initialize Vulkan!
library unloaded
quit
rt-graphics-device-2: exit 0
rt-graphics-device-2: vulkan validation errors: 0
rt-graphics-device-2: openxr valid usage messages: 0
-- rt-graphics-device-lookup
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 6: Failed to get xrGetVulkanGraphicsDevice2KHR, result: -7
log 5: Vulkan: Failed to determine a suitable physical device
device: error: No supported SDL_GPU backend found!
library unloaded
quit
rt-graphics-device-lookup: exit 0
rt-graphics-device-lookup: vulkan validation errors: 0
rt-graphics-device-lookup: openxr valid usage messages: 0
-- rt-vulkan-device
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
log 6: Failed to create OpenXR Vulkan logical device, result -2, 0
device: error: Failed to create logical device!
library unloaded
quit
rt-vulkan-device: exit 0
rt-vulkan-device: vulkan validation errors: 0
rt-vulkan-device: openxr valid usage messages: 0
-- rt-vulkan-device-vk
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
device: error: Failed to create logical device!
library unloaded
quit
rt-vulkan-device-vk: exit 0
rt-vulkan-device-vk: vulkan validation errors: 0
rt-vulkan-device-vk: openxr valid usage messages: 0
-- rt-vulkan-device-lookup
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
log 3: Failed to get xrCreateVulkanDeviceKHR
device: error: Failed to create logical device!
library unloaded
quit
rt-vulkan-device-lookup: exit 0
rt-vulkan-device-lookup: vulkan validation errors: 0
rt-vulkan-device-lookup: openxr valid usage messages: 0
-- rt-session
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
device: ok (vulkan)
session: error -2: 
device destroyed
destroy instance: 0
library unloaded
quit
rt-session: exit 0
rt-session: vulkan validation errors: 0
rt-session: openxr valid usage messages: 0
-- rt-formats
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
device: ok (vulkan)
session: ok
formats: failed
destroy session: 0
device destroyed
destroy instance: 0
library unloaded
quit
rt-formats: exit 0
rt-formats: vulkan validation errors: 0
rt-formats: openxr valid usage messages: 0
-- rt-formats-2
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
device: ok (vulkan)
session: ok
formats: failed
destroy session: 0
device destroyed
destroy instance: 0
library unloaded
quit
rt-formats-2: exit 0
rt-formats-2: vulkan validation errors: 0
rt-formats-2: openxr valid usage messages: 0
-- rt-swapchain
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
device: ok (vulkan)
session: ok
formats: 52 4 53 12 29 60
swapchain: error -2: 
destroy session: 0
device destroyed
destroy instance: 0
library unloaded
quit
rt-swapchain: exit 0
rt-swapchain: vulkan validation errors: 0
rt-swapchain: openxr valid usage messages: 0
-- rt-images
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
device: ok (vulkan)
session: ok
formats: 52 4 53 12 29 60
swapchain: error -2: 
destroy session: 0
device destroyed
destroy instance: 0
library unloaded
quit
rt-images: exit 0
rt-images: vulkan validation errors: 0
rt-images: openxr valid usage messages: 0
-- rt-images-2
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
device: ok (vulkan)
session: ok
formats: 52 4 53 12 29 60
swapchain: error -2: 
destroy session: 0
device destroyed
destroy instance: 0
library unloaded
quit
rt-images-2: exit 0
rt-images-2: vulkan validation errors: 0
rt-images-2: openxr valid usage messages: 0
-- rt-destroy-images
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
device: ok (vulkan)
session: ok
formats: 52 4 53 12 29 60
swapchain: ok
destroy swapchain: error -2: 
destroy session: 0
device destroyed
destroy instance: 0
library unloaded
quit
rt-destroy-images: exit 0
rt-destroy-images: vulkan validation errors: 0
rt-destroy-images: openxr valid usage messages: 0
-- rt-destroy-swapchain
init: ok (offscreen)
load library: ok
log 4: Found XR_KHR_vulkan_enable2 extension
log 4: Found XR_KHR_vulkan_enable2 extension
device: ok (vulkan)
session: ok
formats: 52 4 53 12 29 60
swapchain: ok
destroy swapchain: error -2: 
destroy session: 0
device destroyed
destroy instance: 0
library unloaded
quit
rt-destroy-swapchain: exit 0
rt-destroy-swapchain: vulkan validation errors: 0
rt-destroy-swapchain: openxr valid usage messages: 0
";

    /// How a scenario reaches the test runtime: through the OpenXR loader
    /// (`XR_RUNTIME_JSON` names a manifest of it), or in the loader's place
    /// (`SDL_OPENXR_LIBRARY` names it).
    #[derive(Clone, Copy)]
    enum Via {
        Loader,
        Hint,
    }

    /// The test runtime's scenarios: their names, how they reach it, and its
    /// failure list (`SDL3_OPENXR_TEST_RUNTIME_FAIL`: a call of the
    /// runtime's that fails, all of them or only the `@N`th; in a device's
    /// creation the first instance is PrepareDriver's, the second
    /// CreateDevice's).
    const RUNTIME_SCENARIOS: &[(&str, Via, &str)] = &[
        ("rt-main", Via::Loader, ""),
        ("rt-loader", Via::Loader, ""),
        (
            "rt-negotiate",
            Via::Loader,
            "xrNegotiateLoaderRuntimeInterface",
        ),
        ("rt-hint", Via::Hint, ""),
        ("rt-no-extension", Via::Hint, "XR_KHR_vulkan_enable2"),
        (
            "rt-extensions",
            Via::Hint,
            "xrEnumerateInstanceExtensionProperties",
        ),
        (
            "rt-extensions-2",
            Via::Hint,
            "xrEnumerateInstanceExtensionProperties@2",
        ),
        (
            "rt-extensions-3",
            Via::Hint,
            "xrEnumerateInstanceExtensionProperties@3",
        ),
        ("rt-api-layers", Via::Hint, "xrEnumerateApiLayerProperties"),
        ("rt-instance", Via::Hint, "xrCreateInstance"),
        ("rt-instance-2", Via::Hint, "xrCreateInstance@2"),
        (
            "rt-symbols",
            Via::Hint,
            "xrGetInstanceProcAddr:xrDestroySwapchain",
        ),
        (
            "rt-symbols-2",
            Via::Hint,
            "xrGetInstanceProcAddr:xrDestroySwapchain@2",
        ),
        ("rt-system", Via::Hint, "xrGetSystem"),
        ("rt-system-2", Via::Hint, "xrGetSystem@2"),
        (
            "rt-requirements",
            Via::Hint,
            "xrGetVulkanGraphicsRequirements2KHR",
        ),
        (
            "rt-requirements-2",
            Via::Hint,
            "xrGetVulkanGraphicsRequirements2KHR@2",
        ),
        (
            "rt-requirements-lookup",
            Via::Hint,
            "xrGetInstanceProcAddr:xrGetVulkanGraphicsRequirements2KHR",
        ),
        ("rt-vulkan-instance", Via::Hint, "xrCreateVulkanInstanceKHR"),
        (
            "rt-vulkan-instance-2",
            Via::Hint,
            "xrCreateVulkanInstanceKHR@2",
        ),
        (
            "rt-vulkan-instance-vk",
            Via::Hint,
            "xrCreateVulkanInstanceKHR=vk:-3",
        ),
        (
            "rt-vulkan-instance-lookup",
            Via::Hint,
            "xrGetInstanceProcAddr:xrCreateVulkanInstanceKHR",
        ),
        (
            "rt-graphics-device",
            Via::Hint,
            "xrGetVulkanGraphicsDevice2KHR",
        ),
        (
            "rt-graphics-device-2",
            Via::Hint,
            "xrGetVulkanGraphicsDevice2KHR@2",
        ),
        (
            "rt-graphics-device-lookup",
            Via::Hint,
            "xrGetInstanceProcAddr:xrGetVulkanGraphicsDevice2KHR",
        ),
        ("rt-vulkan-device", Via::Hint, "xrCreateVulkanDeviceKHR"),
        (
            "rt-vulkan-device-vk",
            Via::Hint,
            "xrCreateVulkanDeviceKHR=vk:-3",
        ),
        (
            "rt-vulkan-device-lookup",
            Via::Hint,
            "xrGetInstanceProcAddr:xrCreateVulkanDeviceKHR",
        ),
        ("rt-session", Via::Hint, "xrCreateSession"),
        ("rt-formats", Via::Hint, "xrEnumerateSwapchainFormats"),
        ("rt-formats-2", Via::Hint, "xrEnumerateSwapchainFormats@2"),
        ("rt-swapchain", Via::Hint, "xrCreateSwapchain"),
        ("rt-images", Via::Hint, "xrEnumerateSwapchainImages"),
        ("rt-images-2", Via::Hint, "xrEnumerateSwapchainImages@2"),
        (
            "rt-destroy-images",
            Via::Hint,
            "xrEnumerateSwapchainImages@3",
        ),
        ("rt-destroy-swapchain", Via::Hint, "xrDestroySwapchain"),
    ];

    /// The test runtime's scenarios whose device upstream fails to make
    /// without setting an error (`SDL_GetError()` says what failed before):
    /// the trace says `failed`, and the Rust's error is a `rust:` line.
    const DEVICE_FAILS_WITHOUT_ERROR: &[&str] = &[
        "rt-extensions-3",
        "rt-instance-2",
        "rt-symbols-2",
        "rt-system-2",
        "rt-requirements-2",
    ];

    pub(super) fn main() -> ExitCode {
        if let Ok(scenario) = std::env::var(SCENARIO) {
            return child(&scenario);
        }

        let args: Vec<String> = std::env::args().collect();
        let run_c = args
            .iter()
            .position(|a| a == "--run-c")
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from);

        let program = match &run_c {
            Some(path) => path.clone(),
            None => std::env::current_exe().expect("the program's path"),
        };

        // (each part skips without its capability; with --run-c, the
        // reference's traces are printed, not compared)
        let monado = match missing_openxr() {
            Some(why) => {
                skip(PROGRAM, "openxr", &why);
                true
            }
            None => monado_scenarios(&program, run_c.is_some()),
        };
        let runtime = match missing_openxr_runtime() {
            Some(why) => {
                skip(PROGRAM, "openxr-runtime", &why);
                true
            }
            None => runtime_scenarios(&program, run_c.is_some()),
        };
        if monado && runtime {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }

    /// A new trace, or `None` (a Rust-only failure) when the layouts don't
    /// match the C's.
    fn new_trace() -> Option<Trace> {
        let mut t = Trace::new();
        #[cfg(target_pointer_width = "64")]
        if let Some(difference) = xr::check_layouts() {
            t.rust(false, format!("layouts: {difference}"));
            t.finish(PROGRAM, EXPECTED);
            return None;
        }
        Some(t)
    }

    /// A scenario's child, its lines in the trace.
    fn trace_child(
        t: &mut Trace,
        program: &Path,
        scenario: &str,
        dir: &Path,
        env: &[(&str, &str)],
    ) {
        let out = run_child(program, scenario, dir, env, Duration::from_secs(300));
        for line in out.trace {
            t.line(line);
        }
        t.line(format!("{scenario}: {}", out.status));
        t.line(format!(
            "{scenario}: vulkan validation errors: {}",
            out.validation.len()
        ));
        t.line(format!(
            "{scenario}: openxr valid usage messages: {}",
            out.openxr_usage.len()
        ));
        for message in out.validation.iter().chain(&out.openxr_usage).take(30) {
            println!("{scenario}: validation: {message}");
        }
        // (the Rust's errors where upstream sets none, and the C's lines)
        for line in out.stdout.lines() {
            if line.starts_with("rust: ") || line.starts_with("c: ") {
                println!("{scenario}: {line}");
            }
        }
        if out.status != "exit 0" {
            println!("{scenario}: the child's output:\n{}", out.stdout);
            println!("{scenario}: the child's errors:\n{}", out.stderr);
        }
    }

    /// The scenarios on Monado: whether the trace is upstream's (or, with
    /// `--run-c`, printed).
    fn monado_scenarios(program: &Path, run_c: bool) -> bool {
        let Some(mut t) = new_trace() else {
            return false;
        };
        for &(scenario, with_monado, env) in SCENARIOS {
            t.line(format!("-- {scenario}"));
            let dir = RuntimeDir::new("gpu-openxr");
            let monado = if with_monado {
                match Monado::start(&dir.0) {
                    Ok(monado) => Some(monado),
                    Err(e) => {
                        t.rust(false, format!("{scenario}: {e}"));
                        continue;
                    }
                }
            } else {
                None
            };
            trace_child(&mut t, program, scenario, &dir.0, env);
            drop(monado);
        }
        run_c || t.finish(PROGRAM, EXPECTED)
    }

    /// The scenarios on the test runtime: whether the trace is upstream's
    /// (or, with `--run-c`, printed).
    fn runtime_scenarios(program: &Path, run_c: bool) -> bool {
        let Some(mut t) = new_trace() else {
            return false;
        };
        let library = test_runtime().expect("the test runtime");
        for &(scenario, via, fail) in RUNTIME_SCENARIOS {
            t.line(format!("-- {scenario}"));
            let dir = RuntimeDir::new("gpu-openxr");
            let manifest = write_test_runtime_manifest(&dir.0, &library);
            let manifest = manifest.to_string_lossy();
            let library = library.to_string_lossy();
            let mut env = vec![(TEST_RUNTIME_FAIL, fail)];
            match via {
                Via::Loader => env.push(("XR_RUNTIME_JSON", &manifest)),
                Via::Hint => env.push(("SDL_OPENXR_LIBRARY", &library)),
            }
            trace_child(&mut t, program, scenario, &dir.0, &env);
        }
        run_c || t.finish(PROGRAM, EXPECTED_RUNTIME)
    }

    /// A trace line of the child.
    fn tr(line: impl AsRef<str>) {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{PREFIX}{}", line.as_ref());
        let _ = out.flush();
    }

    /// An OpenXR entry point's error as the C program prints it: the
    /// result and SDL's error (empty when there's none).
    fn xr_error(e: &XrError) -> String {
        format!(
            "error {}: {}",
            e.result().0,
            e.error().map_or("", |e| e.message())
        )
    }

    /// A scenario, in the child.
    fn child(scenario: &str) -> ExitCode {
        sdl3::log::set_output(|record| {
            if record.category == Category::Gpu {
                tr(format!(
                    "log {}: {}",
                    record.priority as i32, record.message
                ));
            }
        });
        sdl3::log::set_priority(Category::Gpu, Priority::Verbose);

        match init::init(InitFlags::VIDEO) {
            Ok(()) => tr(format!(
                "init: ok ({})",
                sdl3::video::current_video_driver().unwrap_or("?")
            )),
            Err(e) => {
                tr(format!("init: error: {e}"));
                return ExitCode::FAILURE;
            }
        }

        match scenario {
            "no-xr" => no_xr(),
            "no-runtime" | "no-service" => no_service(),
            "main" | "rt-main" => main_scenario(),
            _ if scenario.starts_with("rt-") => runtime_steps(scenario),
            _ => panic!("unknown scenario {scenario}"),
        }

        init::quit();
        tr("quit");
        ExitCode::SUCCESS
    }

    /// The creation properties of an OpenXR device (SPIR-V, no verbose
    /// logs).
    fn xr_props(debug_mode: bool) -> Properties {
        let props = Properties::new();
        props
            .set(gpu::PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN, true)
            .unwrap();
        props
            .set(gpu::PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN, debug_mode)
            .unwrap();
        props
            .set(gpu::PROP_GPU_DEVICE_CREATE_VERBOSE_BOOLEAN, false)
            .unwrap();
        props
            .set(gpu::PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN, true)
            .unwrap();
        props
    }

    /// A device made without OpenXR, whose OpenXR entry points fail.
    fn no_xr() {
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
        let device = match Device::with_properties(&props) {
            Ok(device) => device,
            Err(e) => {
                tr(format!("device: error: {e}"));
                return;
            }
        };
        tr(format!("device: ok ({})", device.driver()));

        match device.create_xr_session(&XrSessionCreateInfo::default()) {
            Ok(_) => tr("session: ok"),
            Err(e) => tr(format!("session: {}", xr_error(&e))),
        }
        match device.xr_swapchain_formats(1) {
            Ok(formats) => tr(format!("formats: {}", formats.len())),
            Err(e) => tr(format!("formats: error: {}", e.message())),
        }
        let createinfo = XrSwapchainCreateInfo {
            usage_flags: XrSwapchainUsageFlags::COLOR_ATTACHMENT,
            sample_count: 1,
            width: 16,
            height: 16,
            face_count: 1,
            array_size: 1,
            mip_count: 1,
            ..Default::default()
        };
        match device.create_xr_swapchain(1, &createinfo, TextureFormat::R8G8B8A8_UNORM) {
            Ok(_) => tr("swapchain: ok"),
            Err(e) => tr(format!("swapchain: {}", xr_error(&e))),
        }
        drop(device);
        tr("device destroyed");
    }

    /// An OpenXR device that can't be made (no runtime, or no service).
    fn no_service() {
        let props = xr_props(true);
        tr(format!("supports xr: {}", gpu::supports_properties(&props)));
        match Device::with_properties(&props) {
            Ok(_) => tr("device: ok"),
            Err(e) => tr(format!("device: error: {e}")),
        }
    }

    /// A device on the test runtime and its OpenXR objects, step by step,
    /// each step traced: the session, the swapchain formats, a swapchain
    /// and its destruction, the session's and the instance's (the
    /// runtime's injected failure decides where it stops). The program
    /// holds a reference to the OpenXR library of its own, so that the
    /// runtime, loaded in the loader's place, stays loaded between
    /// PrepareDriver's instance and CreateDevice's (its calls are counted
    /// from its loading).
    fn runtime_steps(scenario: &str) {
        match gpu::openxr_load_library() {
            Ok(()) => tr("load library: ok"),
            Err(e) => {
                tr(format!("load library: error: {e}"));
                return;
            }
        }
        steps_on_device(scenario);
        gpu::openxr_unload_library();
        tr("library unloaded");
    }

    /// [`runtime_steps`]'s device and objects.
    fn steps_on_device(scenario: &str) {
        let device = match Device::with_properties(&xr_props(false)) {
            Ok(device) => device,
            Err(e) => {
                if DEVICE_FAILS_WITHOUT_ERROR.contains(&scenario) {
                    tr("device: failed");
                    println!("rust: device: {e}");
                } else {
                    tr(format!("device: error: {e}"));
                }
                return;
            }
        };
        tr(format!("device: ok ({})", device.driver()));
        let instance = device.xr_instance().unwrap_or(0);
        let gipa = match gpu::openxr_get_xr_get_instance_proc_addr() {
            Ok(gipa) => gipa,
            Err(e) => {
                tr(format!("xrGetInstanceProcAddr: error: {e}"));
                return;
            }
        };
        let x = Xr::load(gipa, instance);

        match device.create_xr_session(&XrSessionCreateInfo::default()) {
            Ok(session) => {
                tr("session: ok");
                swapchain_steps(&device, session);
                // SAFETY: the runtime's function, with the session.
                tr(format!("destroy session: {}", unsafe {
                    (x.xrDestroySession)(session)
                }));
            }
            Err(e) => tr(format!("session: {}", xr_error(&e))),
        }
        drop(device);
        tr("device destroyed");
        // SAFETY: the runtime's function, with the device's instance.
        tr(format!("destroy instance: {}", unsafe {
            (x.xrDestroyInstance)(instance)
        }));
    }

    /// [`runtime_steps`]'s formats and swapchain.
    fn swapchain_steps(device: &Device, session: u64) {
        match device.xr_swapchain_formats(session) {
            Ok(formats) => tr(format!(
                "formats: {}",
                formats
                    .iter()
                    .map(|f| f.0.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            )),
            Err(e) => {
                // (upstream sets no error when the runtime's call fails)
                tr("formats: failed");
                println!("rust: formats: {e}");
                return;
            }
        }
        let createinfo = XrSwapchainCreateInfo {
            usage_flags: XrSwapchainUsageFlags::COLOR_ATTACHMENT
                | XrSwapchainUsageFlags::TRANSFER_SRC,
            sample_count: 1,
            width: 32,
            height: 32,
            face_count: 1,
            array_size: 2,
            mip_count: 1,
            ..Default::default()
        };
        let swapchain =
            match device.create_xr_swapchain(session, &createinfo, TextureFormat::R8G8B8A8_UNORM) {
                Ok(swapchain) => swapchain,
                Err(e) => {
                    tr(format!("swapchain: {}", xr_error(&e)));
                    return;
                }
            };
        // (the image count isn't traced: the C would ask the runtime, a call
        // the failure list counts)
        tr("swapchain: ok");
        println!("rust: swapchain: {} images", swapchain.textures().len());
        match swapchain.destroy() {
            Ok(()) => tr("destroy swapchain: ok"),
            Err(e) => tr(format!("destroy swapchain: {}", xr_error(&e))),
        }
    }

    /// The swapchains the main scenario makes: name, size, layers, usage.
    fn swapchain_specs() -> [(&'static str, u32, u32, XrSwapchainUsageFlags); 3] {
        use XrSwapchainUsageFlags as U;
        [
            ("one layer", 64, 1, U::COLOR_ATTACHMENT | U::TRANSFER_SRC),
            ("two layers", 32, 2, U::COLOR_ATTACHMENT | U::TRANSFER_SRC),
            ("no color attachment", 16, 1, U::SAMPLED | U::TRANSFER_SRC),
        ]
    }

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

    /// The device on Monado: its creation, the session, the swapchains, the
    /// frames, the destruction.
    fn main_scenario() {
        tr(format!(
            "supports xr: {}",
            gpu::supports_properties(&xr_props(true))
        ));

        let handheld = xr_props(false);
        handheld
            .set(gpu::PROP_GPU_DEVICE_CREATE_XR_FORM_FACTOR_NUMBER, 2i64)
            .unwrap();
        match Device::with_properties(&handheld) {
            Ok(_) => tr("handheld: ok"),
            Err(e) => tr(format!("handheld: error: {e}")),
        }

        let unknown = xr_props(false);
        unknown
            .set(gpu::PROP_GPU_DEVICE_CREATE_XR_EXTENSION_COUNT_NUMBER, 1i64)
            .unwrap();
        unknown
            .set_any(
                gpu::PROP_GPU_DEVICE_CREATE_XR_EXTENSION_NAMES_POINTER,
                vec!["XR_SDL3_test_unknown".to_owned()],
            )
            .unwrap();
        match Device::with_properties(&unknown) {
            // (C sets no error there: its SDL_GetError() isn't traced)
            Ok(_) => tr("unknown extension: ok"),
            Err(e) => {
                tr("unknown extension: failed");
                println!("rust: unknown extension: {e}");
            }
        }

        let props = xr_props(true);
        props
            .set(gpu::PROP_GPU_DEVICE_CREATE_XR_EXTENSION_COUNT_NUMBER, 1i64)
            .unwrap();
        props
            .set_any(
                gpu::PROP_GPU_DEVICE_CREATE_XR_EXTENSION_NAMES_POINTER,
                vec!["XR_EXT_debug_utils".to_owned()],
            )
            .unwrap();
        props
            .set(gpu::PROP_GPU_DEVICE_CREATE_XR_LAYER_COUNT_NUMBER, 1i64)
            .unwrap();
        props
            .set_any(
                gpu::PROP_GPU_DEVICE_CREATE_XR_LAYER_NAMES_POINTER,
                vec![XR_VALIDATION_LAYER.to_owned()],
            )
            .unwrap();
        props
            .set(
                gpu::PROP_GPU_DEVICE_CREATE_XR_APPLICATION_NAME_STRING,
                "gpu_openxr",
            )
            .unwrap();
        props
            .set(
                gpu::PROP_GPU_DEVICE_CREATE_XR_APPLICATION_VERSION_NUMBER,
                3i64,
            )
            .unwrap();
        props
            .set(
                gpu::PROP_GPU_DEVICE_CREATE_XR_ENGINE_NAME_STRING,
                "gpu_openxr_engine",
            )
            .unwrap();
        props
            .set(gpu::PROP_GPU_DEVICE_CREATE_XR_ENGINE_VERSION_NUMBER, 5i64)
            .unwrap();
        let device = match Device::with_properties(&props) {
            Ok(device) => device,
            Err(e) => {
                tr(format!("device: error: {e}"));
                return;
            }
        };
        tr(format!("device: ok ({})", device.driver()));
        let instance = device.xr_instance().unwrap_or(0);
        let system_id = device.xr_system_id().unwrap_or(0);
        tr(format!(
            "instance: {}, system: {}",
            if instance != 0 { "non-null" } else { "null" },
            if system_id != 0 { "non-null" } else { "null" }
        ));
        let name = device
            .properties()
            .get_string(gpu::PROP_GPU_DEVICE_NAME_STRING)
            .unwrap_or_default();
        tr(format!(
            "device name: {}",
            name.split_whitespace().next().unwrap_or("")
        ));

        let gipa = match gpu::openxr_get_xr_get_instance_proc_addr() {
            Ok(gipa) => {
                tr("xrGetInstanceProcAddr: ok");
                gipa
            }
            Err(e) => {
                tr(format!("xrGetInstanceProcAddr: error: {e}"));
                return;
            }
        };
        // SAFETY: the loader's xrGetInstanceProcAddr.
        let gipa_fn: xr::PfnGetInstanceProcAddr = unsafe { std::mem::transmute(gipa) };
        let (result, _) = xr::proc_addr(gipa_fn, instance, "xrCreateDebugUtilsMessengerEXT");
        tr(format!("xrCreateDebugUtilsMessengerEXT: {result}"));
        let x = Xr::load(gipa, instance);

        let session = match device.create_xr_session(&XrSessionCreateInfo::default()) {
            Ok(session) => {
                tr("session: ok");
                session
            }
            Err(e) => {
                tr(format!("session: {}", xr_error(&e)));
                return;
            }
        };

        match device.xr_swapchain_formats(session) {
            Ok(formats) => tr(format!(
                "formats: {}",
                formats
                    .iter()
                    .map(|f| f.0.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            )),
            Err(e) => tr(format!("formats: error: {}", e.message())),
        }

        let mut swapchains: Vec<XrSwapchain> = Vec::new();
        for (name, size, layers, usage) in swapchain_specs() {
            let createinfo = XrSwapchainCreateInfo {
                usage_flags: usage,
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
                    tr(format!(
                        "swapchain {name}: ok, {} images",
                        swapchain.textures().len()
                    ));
                    swapchains.push(swapchain);
                }
                Err(e) => {
                    tr(format!("swapchain {name}: {}", xr_error(&e)));
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
        let result = unsafe { (x.xrCreateReferenceSpace)(session, &space_info, &mut space) };
        tr(format!("reference space: {result}"));

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
                tr(format!("state {state}"));
                match state {
                    xr::XR_SESSION_STATE_READY => {
                        let begin = xr::XrSessionBeginInfo {
                            ty: xr::XR_TYPE_SESSION_BEGIN_INFO,
                            next: std::ptr::null(),
                            primary_view_configuration_type:
                                xr::XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO,
                        };
                        // SAFETY: the runtime's function and the info.
                        let result = unsafe { (x.xrBeginSession)(session, &begin) };
                        tr(format!("begin session: {result}"));
                        running = result == 0;
                    }
                    xr::XR_SESSION_STATE_STOPPING => {
                        // SAFETY: the runtime's function.
                        let result = unsafe { (x.xrEndSession)(session) };
                        tr(format!("end session: {result}"));
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
                let result = (x.xrWaitFrame)(session, &wait, &mut frame_state);
                assert_eq!(result, 0, "xrWaitFrame");
                let begin = xr::XrHeader::new(xr::XR_TYPE_FRAME_BEGIN_INFO);
                let result = (x.xrBeginFrame)(session, &begin);
                assert!(result >= 0, "xrBeginFrame {result}");
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
                    let result = (x.xrAcquireSwapchainImage)(handle, &acquire, index);
                    assert_eq!(result, 0, "xrAcquireSwapchainImage");
                    let result = (x.xrWaitSwapchainImage)(handle, &wait);
                    assert_eq!(result, 0, "xrWaitSwapchainImage");
                }
            }

            let mut cb = device.acquire_command_buffer().expect("a command buffer");
            for (k, &index) in indices.iter().enumerate() {
                let texture = &swapchains[k].textures()[index as usize];
                for layer in 0..swapchain_specs()[k].2 {
                    let mut target = ColorTargetInfo::new(texture);
                    target.layer_or_depth_plane = layer;
                    target.clear_color = clear_color(k, layer, frame);
                    target.load_op = LoadOp::Clear;
                    target.store_op = StoreOp::Store;
                    match cb.begin_render_pass(&[target], None) {
                        Ok(pass) => pass.end(),
                        Err(e) => tr(format!("render pass {k} {layer}: error: {e}")),
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
                tr(format!(
                    "frame {frame}: one layer {} {}; two layers {} {}, {} {}",
                    pixel(&mapped[0..]),
                    pixel(&mapped[last(0, one)..]),
                    pixel(&mapped[one as usize..]),
                    pixel(&mapped[last(one, two)..]),
                    pixel(&mapped[(one + two) as usize..]),
                    pixel(&mapped[last(one + two, two)..]),
                ));
            }

            for swapchain in &swapchains[..2] {
                let release = xr::XrHeader::new(xr::XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO);
                // SAFETY: the runtime's function and the info.
                let result = unsafe { (x.xrReleaseSwapchainImage)(swapchain.handle(), &release) };
                assert_eq!(result, 0, "xrReleaseSwapchainImage");
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
            let result = unsafe { (x.xrEndFrame)(session, &end) };
            assert_eq!(result, 0, "xrEndFrame");

            if traced_frame {
                traced += 1;
                if traced == TRACED_FRAMES && !exit_requested {
                    // SAFETY: the runtime's function.
                    let result = unsafe { (x.xrRequestExitSession)(session) };
                    tr(format!("request exit: {result}"));
                    exit_requested = true;
                }
            }
        }
        tr(format!("session done: {done}"));

        for (swapchain, (name, ..)) in swapchains.into_iter().zip(swapchain_specs()) {
            match swapchain.destroy() {
                Ok(()) => tr(format!("destroy swapchain {name}: ok")),
                Err(e) => tr(format!("destroy swapchain {name}: {}", xr_error(&e))),
            }
        }
        drop(readback);
        // SAFETY: the runtime's functions, with the space and the session.
        unsafe {
            tr(format!("destroy space: {}", (x.xrDestroySpace)(space)));
            tr(format!(
                "destroy session: {}",
                (x.xrDestroySession)(session)
            ));
        }
        drop(device);
        tr("device destroyed");
        // SAFETY: the runtime's function, with the device's instance (the
        // application's to destroy, after the device).
        tr(format!("destroy instance: {}", unsafe {
            (x.xrDestroyInstance)(instance)
        }));
    }
}
