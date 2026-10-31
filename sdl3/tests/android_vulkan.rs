// Tests of Vulkan on the android video driver: surfaces on its windows,
// the Vulkan renderer there, and the GPU API's Vulkan backend.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, whose main
//! function traces, against upstream's C SDL:
//!
//! * the library: the instance extensions the driver asks for, Android's
//!   loader (`libvulkan.so`) loaded and its `vkGetInstanceProcAddr`; an
//!   instance made with those extensions: its physical devices (the
//!   emulator's SwiftShader), their queue families and the features the
//!   GPU API asks for;
//! * a Vulkan window: its `ANativeWindow`, no EGL surface; a surface on
//!   it, `SDL_Vulkan_GetPresentationSupport()` and Vulkan's own answer,
//!   the surface's capabilities (its extent: the surface's size), a second
//!   surface on the same window (refused: the native window is in use);
//!   the surface destroyed;
//! * the Vulkan renderer: its name, output size, maximum texture size,
//!   texture formats, vsync and Vulkan properties, the scenes the other
//!   renderers' tests draw (`uikit_support::scenes`, each read back and
//!   hashed, and compared with the software renderer), the custom blend
//!   mode's pixels, a texture of a hardware buffer
//!   (`SDL_PIXELFORMAT_EXTERNAL_OES`: an `AHardwareBuffer` of the
//!   pattern), vsync 1 and 0, and a trip to the background (the swapchain
//!   made again on the new surface) with a scene drawn again;
//! * the GPU API's Vulkan backend (which SwiftShader's features allow): a
//!   device, a window claimed (its swapchain's format, present modes and
//!   compositions), frames cleared and presented (their swapchain
//!   textures' sizes), released, and the "gpu" renderer on the backend;
//! * last, an OpenGL window, whose EGL surface makes Vulkan refuse a
//!   surface there, then a Vulkan window again, and its surface (refused
//!   too: EGL keeps the native window).
//!
//! The expected trace is upstream's C SDL's, from the same program in C on
//! the same emulator (API 35, Google APIs, x86-64), built with the Rust
//! renderer's fixes of upstream's (as the OpenGL ES 2 references are): the
//! pre-rotated swapchain's viewport, clip rectangle and reads (the
//! emulator's surface is rotated 90 degrees for the landscape activity:
//! upstream reads its scenes elsewhere), and the formats of
//! `VK_EXT_4444_formats` left out when the device isn't made with it
//! (both the `FIXME (upstream)` and `Note (upstream)` of the renderer).
//! Elsewhere this program does nothing.

#[cfg(not(target_os = "android"))]
fn main() {}

/// The runner's words: run as an app.
#[cfg(target_os = "android")]
#[used]
#[link_section = ".sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(target_os = "android")]
mod android_support;

#[cfg(target_os = "android")]
#[path = "uikit_support/scenes.rs"]
mod scenes;

#[cfg(target_os = "android")]
fn main() -> std::process::ExitCode {
    android_support::start();
    let status = android_support::checked(app::main);
    android_support::finish(status)
}

#[cfg(target_os = "android")]
mod app {
    use std::ffi::{c_char, c_void, CStr};

    use sdl3::events::window::WindowFlags;
    use sdl3::events::EventType;
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::render::Renderer;
    use sdl3::video::vulkan;
    use sdl3::video::{self, FColor, Window};

    use super::android_support::check_trace;
    use super::android_support::events::{self as ev, is_app_event};
    use super::scenes::{
        check_custom_blend, clear_and_fill, pattern_color, scene, scenes, t, MISMATCHES, TRACE,
    };

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
init: ok
video driver: android
instance extensions: VK_KHR_surface, VK_KHR_android_surface
load the default library: ok
vkGetInstanceProcAddr: ok
instance: ok
physical devices: 1 (VkResult 0)
physical device 0: api=1.3 type=4 name=SwiftShader Device (Subzero)
physical device 0: queue families: 0x7x2
physical device 0: features: independentBlend=1 imageCubeArray=1 depthClamp=1 shaderClipDistance=1 drawIndirectFirstInstance=1 sampleRateShading=1 samplerAnisotropy=1
vulkan window: ok
vulkan window: android window pointer: yes, android surface pointer: no
surface: ok
physical device 0: presentation support: yes, vulkan's: (0, 1)
physical device 0: capabilities: result=0 images=3..64 extent=2400x1080 (pixels 2400x1080) transform=0x2
second surface: error: vkCreateAndroidSurfaceKHR failed: VK_ERROR_NATIVE_WINDOW_IN_USE_KHR, was the window created with SDL_WINDOW_VULKAN?
surface destroyed
instance destroyed, library unloaded
renderer window: ok
renderer: ok
renderer window: flags=0x10000600
renderer: name=vulkan output=2400x1080 pixels=2400x1080 max_texture_size=16384 vsync=0
renderer: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_ABGR2101010, SDL_PIXELFORMAT_RGBA64_FLOAT, SDL_PIXELFORMAT_RGB565, SDL_PIXELFORMAT_BGR565, SDL_PIXELFORMAT_RGBA5551, SDL_PIXELFORMAT_BGRA5551, SDL_PIXELFORMAT_ARGB1555, SDL_PIXELFORMAT_RGBA4444, SDL_PIXELFORMAT_BGRA4444, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21, SDL_PIXELFORMAT_P010, SDL_PIXELFORMAT_I0FL, SDL_PIXELFORMAT_I4FL, SDL_PIXELFORMAT_EXTERNAL_OES
renderer: vulkan: instance=yes surface=yes physical device=yes device=yes graphics queue family=0 present queue family=0 swapchain images=5
scene clear and fill: 64x48 5fe1f24a
scene points and lines: 64x48 75192d12
scene viewport and clip: 64x48 b09f79e5
scene blend modes: 64x48 ebb10095
scene texture SDL_PIXELFORMAT_ARGB8888: 64x48 a2cf2633
scene texture SDL_PIXELFORMAT_ABGR8888: 64x48 a2cf2633
scene texture SDL_PIXELFORMAT_RGB24: 64x48 4fb014c5
scene texture SDL_PIXELFORMAT_XRGB8888: 64x48 4fb014c5
scene streaming texture: 64x48 c2319ec5
scene render target: 64x48 9e5285c5
scene packed SDL_PIXELFORMAT_ABGR2101010: 64x48 e9c8703d
scene packed SDL_PIXELFORMAT_RGBA64_FLOAT: 64x48 bf4cacb5
scene packed SDL_PIXELFORMAT_RGBA128_FLOAT: 64x48 bf4cacb5
scene packed SDL_PIXELFORMAT_RGB565: 64x48 bdd0ee0d
scene packed SDL_PIXELFORMAT_RGBA5551: 64x48 6c4cab9d
scene packed SDL_PIXELFORMAT_ARGB1555: 64x48 6c4cab9d
scene packed SDL_PIXELFORMAT_RGBA4444: 64x48 537d6565
scene palette texture: 64x48 65a75445
scene yuv SDL_PIXELFORMAT_IYUV: 64x48 785ae64d
scene yuv SDL_PIXELFORMAT_YV12: 64x48 785ae64d
scene yuv SDL_PIXELFORMAT_NV12: 64x48 785ae64d
scene yuv SDL_PIXELFORMAT_NV21: 64x48 785ae64d
scene IYUV planes: 64x48 785ae64d
scene NV12 planes: 64x48 785ae64d
scene locked texture: 64x48 76d14245
scene locked NV12 texture: 64x48 785ae64d
scene custom blend mode: 64x48 af879425
scene wrapping geometry: 64x48 dd468f45
scene colored geometry: 64x48 7a2c48ad
scene linear scaling: 64x48 5599895b
scene debug text: 64x48 12491c24
partial read: 8x8 f24b7925
target read: 20x10 da6561f5
scene external OES texture: 64x48 60fcb756
set vsync 1: ok, vsync=1
set vsync 0: ok, vsync=0
trip: app events: WILL_ENTER_BACKGROUND(0,0), DID_ENTER_BACKGROUND(0,0), WILL_ENTER_FOREGROUND(0,0), DID_ENTER_FOREGROUND(0,0)
trip: render events: (none)
after the trip: name=vulkan output=2400x1080 pixels=2400x1080 max_texture_size=16384 vsync=0
after the trip: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_ABGR2101010, SDL_PIXELFORMAT_RGBA64_FLOAT, SDL_PIXELFORMAT_RGB565, SDL_PIXELFORMAT_BGR565, SDL_PIXELFORMAT_RGBA5551, SDL_PIXELFORMAT_BGRA5551, SDL_PIXELFORMAT_ARGB1555, SDL_PIXELFORMAT_RGBA4444, SDL_PIXELFORMAT_BGRA4444, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21, SDL_PIXELFORMAT_P010, SDL_PIXELFORMAT_I0FL, SDL_PIXELFORMAT_I4FL, SDL_PIXELFORMAT_EXTERNAL_OES
after the trip: vulkan: instance=yes surface=yes physical device=yes device=yes graphics queue family=0 present queue family=0 swapchain images=5
scene after the trip: 64x48 5fe1f24a
renderer destroyed
gpu: supports SPIR-V on vulkan: yes
gpu: create vulkan device: ok
gpu: driver=vulkan shader formats=0x2
gpu window: ok
gpu: claim: ok
gpu: window flags=0x600
gpu: swapchain format: 4
gpu: present modes: vsync=yes immediate=no mailbox=yes
gpu: compositions: sdr=yes sdr linear=yes hdr extended linear=yes hdr10=yes
gpu: frame 1: 2400x1080, submitted: ok
gpu: frame 2: 2400x1080, submitted: ok
gpu: frame 3: 2400x1080, submitted: ok
gpu: wait for idle: ok
gpu: released: swapchain format: error: Cannot get swapchain format, window has not been claimed!
gpu: device destroyed
gpu renderer: ok
gpu renderer: name=gpu output=2400x1080 vsync=0
scene gpu clear and fill: 64x48 5fe1f24a
gpu renderer destroyed
after opengl: load the library: ok
opengl window: ok
opengl window: surface: error: vkCreateAndroidSurfaceKHR failed: VK_ERROR_NATIVE_WINDOW_IN_USE_KHR, was the window created with SDL_WINDOW_VULKAN?
vulkan window again: ok
vulkan window again: surface: error: vkCreateAndroidSurfaceKHR failed: VK_ERROR_NATIVE_WINDOW_IN_USE_KHR, was the window created with SDL_WINDOW_VULKAN?
after opengl: instance destroyed, library unloaded
quit
";

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

    fn has(w: &Window, name: &str) -> bool {
        w.properties()
            .ok()
            .and_then(|p| p.get_number(name))
            .is_some_and(|n| n != 0)
    }

    // ------------------------------------------------------------------
    // The bits of Vulkan the program calls itself

    type VkInstance = *mut c_void;
    type VkPhysicalDevice = *mut c_void;
    type PfnVoid = unsafe extern "system" fn();
    type PfnGetInstanceProcAddr =
        unsafe extern "system" fn(VkInstance, *const c_char) -> Option<PfnVoid>;

    /// `VkInstanceCreateInfo`.
    #[repr(C)]
    struct VkInstanceCreateInfo {
        s_type: i32,
        p_next: *const c_void,
        flags: u32,
        p_application_info: *const c_void,
        enabled_layer_count: u32,
        pp_enabled_layer_names: *const *const c_char,
        enabled_extension_count: u32,
        pp_enabled_extension_names: *const *const c_char,
    }

    /// `VkApplicationInfo`.
    #[repr(C)]
    struct VkApplicationInfo {
        s_type: i32,
        p_next: *const c_void,
        p_application_name: *const c_char,
        application_version: u32,
        p_engine_name: *const c_char,
        engine_version: u32,
        api_version: u32,
    }

    /// `VkQueueFamilyProperties`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct VkQueueFamilyProperties {
        queue_flags: u32,
        queue_count: u32,
        timestamp_valid_bits: u32,
        min_image_transfer_granularity: [u32; 3],
    }

    /// `VkSurfaceCapabilitiesKHR`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct VkSurfaceCapabilitiesKHR {
        min_image_count: u32,
        max_image_count: u32,
        current_extent: [u32; 2],
        min_image_extent: [u32; 2],
        max_image_extent: [u32; 2],
        max_image_array_layers: u32,
        supported_transforms: u32,
        current_transform: u32,
        supported_composite_alpha: u32,
        supported_usage_flags: u32,
    }

    /// `VkPhysicalDeviceProperties` (824 bytes), of which the program reads
    /// the API version, the device type and the name.
    #[repr(C, align(8))]
    struct VkPhysicalDeviceProperties([u8; 1024]);

    /// The functions of an instance (or the global ones, for a null one),
    /// through `vkGetInstanceProcAddr`.
    struct Vk {
        get: PfnGetInstanceProcAddr,
    }

    impl Vk {
        fn new() -> Vk {
            let get = vulkan::vulkan_get_vk_get_instance_proc_addr().unwrap();
            // SAFETY: the loader's vkGetInstanceProcAddr.
            Vk {
                get: unsafe { std::mem::transmute::<usize, PfnGetInstanceProcAddr>(get) },
            }
        }

        /// A function of `instance`, as `F`.
        ///
        /// # Safety
        ///
        /// `F` must be the function's pointer type.
        unsafe fn f<F: Copy>(&self, instance: VkInstance, name: &CStr) -> F {
            // SAFETY: the caller's contract.
            unsafe {
                let f =
                    (self.get)(instance, name.as_ptr()).unwrap_or_else(|| panic!("no {name:?}"));
                std::mem::transmute_copy::<PfnVoid, F>(&f)
            }
        }

        /// `vkCreateInstance()` with these extensions: the instance, or
        /// the `VkResult`.
        fn create_instance(&self, extensions: &[&str]) -> Result<VkInstance, i32> {
            let names: Vec<std::ffi::CString> = extensions
                .iter()
                .map(|e| std::ffi::CString::new(*e).unwrap())
                .collect();
            let pointers: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
            // (Vulkan 1.0, as the renderer's)
            let app = VkApplicationInfo {
                s_type: 0, // VK_STRUCTURE_TYPE_APPLICATION_INFO
                p_next: std::ptr::null(),
                p_application_name: std::ptr::null(),
                application_version: 0,
                p_engine_name: std::ptr::null(),
                engine_version: 0,
                api_version: 1 << 22, // VK_API_VERSION_1_0
            };
            let info = VkInstanceCreateInfo {
                s_type: 1, // VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO
                p_next: std::ptr::null(),
                flags: 0,
                p_application_info: (&raw const app).cast(),
                enabled_layer_count: 0,
                pp_enabled_layer_names: std::ptr::null(),
                enabled_extension_count: pointers.len() as u32,
                pp_enabled_extension_names: pointers.as_ptr(),
            };
            type Create = unsafe extern "system" fn(
                *const VkInstanceCreateInfo,
                *const c_void,
                *mut VkInstance,
            ) -> i32;
            let mut instance: VkInstance = std::ptr::null_mut();
            // SAFETY: the global function, with the info above.
            let result = unsafe {
                let create: Create = self.f(std::ptr::null_mut(), c"vkCreateInstance");
                create(&info, std::ptr::null(), &mut instance)
            };
            if result == 0 {
                Ok(instance)
            } else {
                Err(result)
            }
        }

        fn destroy_instance(&self, instance: VkInstance) {
            type Destroy = unsafe extern "system" fn(VkInstance, *const c_void);
            // SAFETY: the instance's function.
            unsafe {
                let destroy: Destroy = self.f(instance, c"vkDestroyInstance");
                destroy(instance, std::ptr::null());
            }
        }

        /// The physical devices, and the `VkResult` of the first call.
        fn physical_devices(&self, instance: VkInstance) -> (Vec<VkPhysicalDevice>, i32) {
            type Enumerate =
                unsafe extern "system" fn(VkInstance, *mut u32, *mut VkPhysicalDevice) -> i32;
            // SAFETY: the instance's function, with room for the count.
            unsafe {
                let enumerate: Enumerate = self.f(instance, c"vkEnumeratePhysicalDevices");
                let mut count = 0;
                let result = enumerate(instance, &mut count, std::ptr::null_mut());
                let mut devices = vec![std::ptr::null_mut(); count as usize];
                enumerate(instance, &mut count, devices.as_mut_ptr());
                devices.truncate(count as usize);
                (devices, result)
            }
        }

        /// The device's API version (major.minor), type and name.
        fn device_properties(&self, instance: VkInstance, device: VkPhysicalDevice) -> String {
            type Get = unsafe extern "system" fn(VkPhysicalDevice, *mut VkPhysicalDeviceProperties);
            let mut props = VkPhysicalDeviceProperties([0; 1024]);
            // SAFETY: the instance's function, with room for the
            // properties.
            unsafe {
                let get: Get = self.f(instance, c"vkGetPhysicalDeviceProperties");
                get(device, &mut props);
            }
            let api = u32::from_ne_bytes(props.0[0..4].try_into().unwrap());
            let device_type = u32::from_ne_bytes(props.0[16..20].try_into().unwrap());
            let name = CStr::from_bytes_until_nul(&props.0[20..276])
                .unwrap()
                .to_string_lossy()
                .into_owned();
            format!(
                "api={}.{} type={device_type} name={name}",
                api >> 22,
                (api >> 12) & 0x3ff
            )
        }

        /// The Vulkan 1.0 features the GPU API's Vulkan backend asks for
        /// by default (`VkPhysicalDeviceFeatures`' 55 `VkBool32`s, of
        /// which these are the 4th, 3rd, 12th, 38th, 11th, 7th and 20th).
        fn gpu_features(&self, instance: VkInstance, device: VkPhysicalDevice) -> String {
            type Get = unsafe extern "system" fn(VkPhysicalDevice, *mut [u32; 55]);
            let mut f = [0u32; 55];
            // SAFETY: the instance's function, with room for the features.
            unsafe {
                let get: Get = self.f(instance, c"vkGetPhysicalDeviceFeatures");
                get(device, &mut f);
            }
            format!(
                "independentBlend={} imageCubeArray={} depthClamp={} shaderClipDistance={} drawIndirectFirstInstance={} sampleRateShading={} samplerAnisotropy={}",
                f[3], f[2], f[11], f[37], f[10], f[6], f[19]
            )
        }

        fn queue_families(
            &self,
            instance: VkInstance,
            device: VkPhysicalDevice,
        ) -> Vec<VkQueueFamilyProperties> {
            type Get =
                unsafe extern "system" fn(VkPhysicalDevice, *mut u32, *mut VkQueueFamilyProperties);
            // SAFETY: the instance's function, with room for the count.
            unsafe {
                let get: Get = self.f(instance, c"vkGetPhysicalDeviceQueueFamilyProperties");
                let mut count = 0;
                get(device, &mut count, std::ptr::null_mut());
                let mut families = vec![VkQueueFamilyProperties::default(); count as usize];
                get(device, &mut count, families.as_mut_ptr());
                families.truncate(count as usize);
                families
            }
        }

        /// `vkGetPhysicalDeviceSurfaceSupportKHR()`: the result and the
        /// answer.
        fn surface_support(
            &self,
            instance: VkInstance,
            device: VkPhysicalDevice,
            family: u32,
            surface: u64,
        ) -> (i32, u32) {
            type Get = unsafe extern "system" fn(VkPhysicalDevice, u32, u64, *mut u32) -> i32;
            let mut supported = 0;
            // SAFETY: the instance's function.
            let result = unsafe {
                let get: Get = self.f(instance, c"vkGetPhysicalDeviceSurfaceSupportKHR");
                get(device, family, surface, &mut supported)
            };
            (result, supported)
        }

        fn surface_capabilities(
            &self,
            instance: VkInstance,
            device: VkPhysicalDevice,
            surface: u64,
        ) -> (i32, VkSurfaceCapabilitiesKHR) {
            type Get = unsafe extern "system" fn(
                VkPhysicalDevice,
                u64,
                *mut VkSurfaceCapabilitiesKHR,
            ) -> i32;
            let mut caps = VkSurfaceCapabilitiesKHR::default();
            // SAFETY: the instance's function.
            let result = unsafe {
                let get: Get = self.f(instance, c"vkGetPhysicalDeviceSurfaceCapabilitiesKHR");
                get(device, surface, &mut caps)
            };
            (result, caps)
        }
    }

    // ------------------------------------------------------------------

    /// The renderer's name, sizes, formats and Vulkan properties.
    fn renderer_state(label: &str, r: &mut Renderer, w: &Window) {
        use sdl3::render::{
            PROP_RENDERER_VULKAN_DEVICE_POINTER,
            PROP_RENDERER_VULKAN_GRAPHICS_QUEUE_FAMILY_INDEX_NUMBER,
            PROP_RENDERER_VULKAN_INSTANCE_POINTER, PROP_RENDERER_VULKAN_PHYSICAL_DEVICE_POINTER,
            PROP_RENDERER_VULKAN_PRESENT_QUEUE_FAMILY_INDEX_NUMBER,
            PROP_RENDERER_VULKAN_SURFACE_NUMBER, PROP_RENDERER_VULKAN_SWAPCHAIN_IMAGE_COUNT_NUMBER,
        };
        let props = r.properties();
        let (ow, oh) = r.output_size().unwrap_or_default();
        let (pw, ph) = w.size_in_pixels().unwrap_or_default();
        t(format!(
            "{label}: name={} output={ow}x{oh} pixels={pw}x{ph} max_texture_size={} vsync={}",
            r.name(),
            props
                .get_number(sdl3::render::PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER)
                .unwrap_or(-1),
            r.vsync()
        ));
        let formats: Vec<&str> = r.texture_formats().iter().map(|f| f.name()).collect();
        t(format!("{label}: formats: {}", formats.join(", ")));
        let set = |name: &str| yes(props.get_number(name).unwrap_or(0) != 0);
        t(format!(
            "{label}: vulkan: instance={} surface={} physical device={} device={} graphics queue family={} present queue family={} swapchain images={}",
            set(PROP_RENDERER_VULKAN_INSTANCE_POINTER),
            set(PROP_RENDERER_VULKAN_SURFACE_NUMBER),
            set(PROP_RENDERER_VULKAN_PHYSICAL_DEVICE_POINTER),
            set(PROP_RENDERER_VULKAN_DEVICE_POINTER),
            props
                .get_number(PROP_RENDERER_VULKAN_GRAPHICS_QUEUE_FAMILY_INDEX_NUMBER)
                .unwrap_or(-1),
            props
                .get_number(PROP_RENDERER_VULKAN_PRESENT_QUEUE_FAMILY_INDEX_NUMBER)
                .unwrap_or(-1),
            props
                .get_number(PROP_RENDERER_VULKAN_SWAPCHAIN_IMAGE_COUNT_NUMBER)
                .unwrap_or(-1),
        ));
    }

    fn surfaces() {
        let extensions = vulkan::vulkan_instance_extensions();
        t(format!(
            "instance extensions: {}",
            extensions
                .as_ref()
                .map_or_else(|e| format!("error: {e}"), |x| x.join(", "))
        ));
        t(format!(
            "load the default library: {}",
            result(&vulkan::vulkan_load_library(None))
        ));
        t(format!(
            "vkGetInstanceProcAddr: {}",
            result(&vulkan::vulkan_get_vk_get_instance_proc_addr())
        ));
        let vk = Vk::new();
        let extensions = extensions.unwrap_or_default();
        let instance = vk.create_instance(&extensions);
        t(format!(
            "instance: {}",
            match &instance {
                Ok(_) => "ok".to_string(),
                Err(r) => format!("VkResult {r}"),
            }
        ));
        let Ok(instance) = instance else {
            return;
        };
        let (devices, enumerated) = vk.physical_devices(instance);
        t(format!(
            "physical devices: {} (VkResult {enumerated})",
            devices.len()
        ));
        for (i, &d) in devices.iter().enumerate() {
            t(format!(
                "physical device {i}: {}",
                vk.device_properties(instance, d)
            ));
            let families: Vec<String> = vk
                .queue_families(instance, d)
                .iter()
                .map(|f| format!("0x{:x}x{}", f.queue_flags, f.queue_count))
                .collect();
            t(format!(
                "physical device {i}: queue families: {}",
                families.join(", ")
            ));
            t(format!(
                "physical device {i}: features: {}",
                vk.gpu_features(instance, d)
            ));
        }

        // A Vulkan window, and a surface on it
        let created = Window::create("android vulkan", 320, 240, WindowFlags::VULKAN);
        t(format!("vulkan window: {}", result(&created)));
        let v = created.expect("the window");
        // (a window wider than tall turns the activity to landscape: the
        // surface's capabilities follow once the surface is turned)
        ev::pump_until(|| v.size_in_pixels().is_ok_and(|(w, h)| w > h), 5000, 300);
        ev::take();
        t(format!(
            "vulkan window: android window pointer: {}, android surface pointer: {}",
            yes(has(&v, video::window::PROP_WINDOW_ANDROID_WINDOW_POINTER)),
            yes(has(&v, video::window::PROP_WINDOW_ANDROID_SURFACE_POINTER))
        ));
        let surface = vulkan::vulkan_create_surface(&v, instance as usize, 0);
        t(format!("surface: {}", result(&surface)));
        if let Ok(surface) = surface {
            for (i, &d) in devices.iter().enumerate() {
                t(format!(
                    "physical device {i}: presentation support: {}, vulkan's: {:?}",
                    vulkan::vulkan_presentation_support(instance as usize, d as usize, 0)
                        .map_or_else(|e| format!("error: {e}"), |s| yes(s).to_string()),
                    vk.surface_support(instance, d, 0, surface)
                ));
                let (result, caps) = vk.surface_capabilities(instance, d, surface);
                let (w, h) = v.size_in_pixels().unwrap_or_default();
                t(format!(
                    "physical device {i}: capabilities: result={result} images={}..{} extent={}x{} (pixels {w}x{h}) transform=0x{:x}",
                    caps.min_image_count,
                    caps.max_image_count,
                    caps.current_extent[0],
                    caps.current_extent[1],
                    caps.current_transform
                ));
            }
            let second = vulkan::vulkan_create_surface(&v, instance as usize, 0);
            t(format!("second surface: {}", result(&second)));
            if let Ok(second) = second {
                vulkan::vulkan_destroy_surface(instance as usize, second, 0);
            }
            vulkan::vulkan_destroy_surface(instance as usize, surface, 0);
            t("surface destroyed");
        }
        v.destroy();
        ev::pump_for(200);
        ev::take();

        vk.destroy_instance(instance);
        vulkan::vulkan_unload_library();
        t("instance destroyed, library unloaded");
    }

    /// An OpenGL window, whose EGL surface has the native window, then a
    /// Vulkan window again (last: what EGL leaves of the native window
    /// stays).
    fn after_opengl() {
        t(format!(
            "after opengl: load the library: {}",
            result(&vulkan::vulkan_load_library(None))
        ));
        let vk = Vk::new();
        let extensions = vulkan::vulkan_instance_extensions().unwrap_or_default();
        let Ok(instance) = vk.create_instance(&extensions) else {
            t("after opengl: no instance");
            return;
        };
        let created = Window::create("android gl", 320, 240, WindowFlags::OPENGL);
        t(format!("opengl window: {}", result(&created)));
        if let Ok(o) = created {
            let surface = vulkan::vulkan_create_surface(&o, instance as usize, 0);
            t(format!("opengl window: surface: {}", result(&surface)));
            if let Ok(surface) = surface {
                vulkan::vulkan_destroy_surface(instance as usize, surface, 0);
            }
            o.destroy();
        }
        ev::pump_for(200);
        ev::take();
        let created = Window::create("android vulkan again", 320, 240, WindowFlags::VULKAN);
        t(format!("vulkan window again: {}", result(&created)));
        if let Ok(v) = created {
            let surface = vulkan::vulkan_create_surface(&v, instance as usize, 0);
            t(format!(
                "vulkan window again: surface: {}",
                result(&surface)
            ));
            if let Ok(surface) = surface {
                vulkan::vulkan_destroy_surface(instance as usize, surface, 0);
            }
            v.destroy();
        }
        ev::pump_for(200);
        ev::take();
        vk.destroy_instance(instance);
        vulkan::vulkan_unload_library();
        t("after opengl: instance destroyed, library unloaded");
    }

    // ------------------------------------------------------------------
    // Android's hardware buffers (API 26: from libandroid.so)

    /// `AHardwareBuffer_Desc`.
    #[repr(C)]
    #[derive(Default)]
    struct AHardwareBufferDesc {
        width: u32,
        height: u32,
        layers: u32,
        format: u32,
        usage: u64,
        stride: u32,
        rfu0: u32,
        rfu1: u64,
    }

    /// `AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM`
    const AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM: u32 = 1;
    /// `AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN`
    const AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN: u64 = 0x30;
    /// `AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE`
    const AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE: u64 = 0x100;

    type Allocate = unsafe extern "C" fn(*const AHardwareBufferDesc, *mut *mut c_void) -> i32;
    type Describe = unsafe extern "C" fn(*const c_void, *mut AHardwareBufferDesc);
    type Lock = unsafe extern "C" fn(*mut c_void, u64, i32, *const c_void, *mut *mut c_void) -> i32;
    type Unlock = unsafe extern "C" fn(*mut c_void, *mut i32) -> i32;
    type Release = unsafe extern "C" fn(*mut c_void);

    /// The functions of `AHardwareBuffer` the program calls.
    struct HardwareBuffers {
        allocate: Allocate,
        describe: Describe,
        lock: Lock,
        unlock: Unlock,
        release: Release,
    }

    fn hardware_buffers() -> Option<HardwareBuffers> {
        // SAFETY: dlopen() and dlsym() of NUL-terminated names, the
        // functions of the NDK's prototypes (the library stays loaded).
        unsafe {
            let lib = libc::dlopen(c"libandroid.so".as_ptr(), libc::RTLD_NOW);
            if lib.is_null() {
                return None;
            }
            let f = |name: &CStr| {
                let f = libc::dlsym(lib, name.as_ptr());
                (!f.is_null()).then_some(f)
            };
            Some(HardwareBuffers {
                allocate: std::mem::transmute::<*mut c_void, Allocate>(f(
                    c"AHardwareBuffer_allocate",
                )?),
                describe: std::mem::transmute::<*mut c_void, Describe>(f(
                    c"AHardwareBuffer_describe",
                )?),
                lock: std::mem::transmute::<*mut c_void, Lock>(f(c"AHardwareBuffer_lock")?),
                unlock: std::mem::transmute::<*mut c_void, Unlock>(f(c"AHardwareBuffer_unlock")?),
                release: std::mem::transmute::<*mut c_void, Release>(f(
                    c"AHardwareBuffer_release",
                )?),
            })
        }
    }

    /// The texture scene (`scenes::texture_scene()`) with an
    /// `SDL_PIXELFORMAT_EXTERNAL_OES` texture on a hardware buffer of the
    /// pattern (RGBA8).
    fn external_oes(r: &mut Renderer) -> sdl3::Result<()> {
        use sdl3::render::{NativeObject, TextureAccess, TextureCreateInfo};
        use sdl3::video::{BlendMode, FRect, PixelFormat, ScaleMode};

        r.set_draw_blend_mode(BlendMode::NONE)?;
        r.set_draw_color(40, 50, 60, 255);
        r.clear()?;
        let hb = hardware_buffers().ok_or_else(|| sdl3::Error::new("no AHardwareBuffer"))?;
        let desc = AHardwareBufferDesc {
            width: 16,
            height: 16,
            layers: 1,
            format: AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM,
            usage: AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN | AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE,
            ..Default::default()
        };
        let mut buffer = std::ptr::null_mut();
        // SAFETY: the description above, and room for the buffer.
        if unsafe { (hb.allocate)(&desc, &mut buffer) } != 0 {
            return Err(sdl3::Error::new("AHardwareBuffer_allocate() failed"));
        }
        let mut described = AHardwareBufferDesc::default();
        let mut pixels = std::ptr::null_mut();
        // SAFETY: the buffer just made, locked for the CPU's writes of its
        // stride's rows, then unlocked.
        unsafe {
            (hb.describe)(buffer, &mut described);
            if (hb.lock)(
                buffer,
                AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN,
                -1,
                std::ptr::null(),
                &mut pixels,
            ) != 0
            {
                (hb.release)(buffer);
                return Err(sdl3::Error::new("AHardwareBuffer_lock() failed"));
            }
            for y in 0..16 {
                for x in 0..16 {
                    let c = pattern_color(x, y);
                    let at = (y as usize * described.stride as usize + x as usize) * 4;
                    let p = pixels.cast::<u8>().add(at);
                    *p = c.r;
                    *p.add(1) = c.g;
                    *p.add(2) = c.b;
                    *p.add(3) = c.a;
                }
            }
            (hb.unlock)(buffer, std::ptr::null_mut());
        }
        let tex = r.create_texture_with(&TextureCreateInfo {
            format: PixelFormat::EXTERNAL_OES,
            access: TextureAccess::Static,
            width: 16,
            height: 16,
            // SAFETY: the buffer, released once the texture is destroyed.
            vulkan_android_hardware_buffer: unsafe { NativeObject::from_raw(buffer as u64) },
            ..Default::default()
        });
        let result = tex.and_then(|tex| {
            let drawn = (|| {
                r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
                r.render_texture(tex, None, Some(&FRect::new(4.0, 4.0, 16.0, 16.0)))?;
                r.set_texture_color_mod(tex, 255, 128, 64)?;
                r.set_texture_alpha_mod(tex, 160)?;
                r.render_texture(tex, None, Some(&FRect::new(24.0, 8.0, 32.0, 32.0)))
            })();
            r.destroy_texture(tex);
            drawn
        });
        // SAFETY: the buffer, which no texture uses now.
        unsafe { (hb.release)(buffer) };
        result
    }

    fn renderer() {
        let created = Window::create("android vulkan renderer", 320, 240, WindowFlags::default());
        t(format!("renderer window: {}", result(&created)));
        let w = created.expect("the window");
        let r = Renderer::for_window(&w, Some("vulkan"));
        t(format!("renderer: {}", result(&r)));
        let Ok(mut r) = r else {
            w.destroy();
            return;
        };
        ev::pump_for(300);
        ev::take();
        t(format!(
            "renderer window: flags=0x{:x}",
            w.flags().map_or(0, |f| f.0)
        ));
        renderer_state("renderer", &mut r, &w);
        scenes(&mut r, "vulkan");
        check_custom_blend(&mut r);
        // (not compared: the software renderer has no such textures)
        scene(&mut r, "external OES texture", None, &external_oes);
        for vsync in [1, 0] {
            t(format!(
                "set vsync {vsync}: {}, vsync={}",
                result(&r.set_vsync(vsync)),
                r.vsync()
            ));
        }

        // A trip to the background and back: a new surface and swapchain
        let events = ev::trip(1500);
        t(format!(
            "trip: app events: {}",
            ev::ordered(&events, |ty| is_app_event(ty)
                && ty != EventType::LOW_MEMORY)
        ));
        t(format!(
            "trip: render events: {}",
            ev::names(&events, |ty| (0x2000..0x2100).contains(&ty.0))
        ));
        renderer_state("after the trip", &mut r, &w);
        scene(&mut r, "after the trip", Some((0, 0)), &clear_and_fill);
        drop(r);
        t("renderer destroyed");
        w.destroy();
        ev::pump_for(200);
        ev::take();
    }

    fn gpu() {
        use sdl3::gpu::{
            ColorTargetInfo, Device, LoadOp, PresentMode, ShaderFormat, SwapchainComposition,
        };

        t(format!(
            "gpu: supports SPIR-V on vulkan: {}",
            yes(sdl3::gpu::supports_shader_formats(
                ShaderFormat::SPIRV,
                Some("vulkan")
            ))
        ));
        let device = Device::new(ShaderFormat::SPIRV, false, Some("vulkan"));
        t(format!("gpu: create vulkan device: {}", result(&device)));
        let Ok(device) = device else {
            return;
        };
        t(format!(
            "gpu: driver={} shader formats=0x{:x}",
            device.driver(),
            device.shader_formats().0
        ));
        let created = Window::create("android gpu", 320, 240, WindowFlags::default());
        t(format!("gpu window: {}", result(&created)));
        let g = created.expect("the window");
        ev::pump_for(300);
        ev::take();
        let claimed = device.claim_window(&g);
        t(format!("gpu: claim: {}", result(&claimed)));
        if claimed.is_ok() {
            t(format!(
                "gpu: window flags=0x{:x}",
                g.flags().map_or(0, |f| f.0)
            ));
            t(format!(
                "gpu: swapchain format: {}",
                match device.swapchain_texture_format(&g) {
                    Ok(f) => f.0.to_string(),
                    Err(e) => format!("error: {e}"),
                }
            ));
            t(format!(
                "gpu: present modes: vsync={} immediate={} mailbox={}",
                yes(device.window_supports_present_mode(&g, PresentMode::Vsync)),
                yes(device.window_supports_present_mode(&g, PresentMode::Immediate)),
                yes(device.window_supports_present_mode(&g, PresentMode::Mailbox))
            ));
            t(format!(
                "gpu: compositions: sdr={} sdr linear={} hdr extended linear={} hdr10={}",
                yes(device.window_supports_swapchain_composition(&g, SwapchainComposition::Sdr)),
                yes(device
                    .window_supports_swapchain_composition(&g, SwapchainComposition::SdrLinear)),
                yes(device.window_supports_swapchain_composition(
                    &g,
                    SwapchainComposition::HdrExtendedLinear
                )),
                yes(device
                    .window_supports_swapchain_composition(&g, SwapchainComposition::Hdr10St2084))
            ));
            let blue = FColor::new(0.0, 0.0, 1.0, 1.0);
            for i in 1..=3 {
                let mut cb = device.acquire_command_buffer().unwrap();
                let texture = cb.wait_and_acquire_swapchain_texture(&g);
                let what = match &texture {
                    Ok(Some(t)) => format!("{}x{}", t.width, t.height),
                    Ok(None) => "no texture".to_string(),
                    Err(e) => format!("error: {e}"),
                };
                if let Ok(Some(texture)) = &texture {
                    let target = ColorTargetInfo {
                        load_op: LoadOp::Clear,
                        clear_color: blue,
                        ..ColorTargetInfo::new(&texture.texture)
                    };
                    cb.begin_render_pass(&[target], None).unwrap().end();
                }
                let submitted = cb.submit();
                t(format!(
                    "gpu: frame {i}: {what}, submitted: {}",
                    result(&submitted)
                ));
            }
            t(format!(
                "gpu: wait for idle: {}",
                result(&device.wait_for_idle())
            ));
            device.release_window(&g);
            t(format!(
                "gpu: released: swapchain format: {}",
                match device.swapchain_texture_format(&g) {
                    Ok(f) => f.0.to_string(),
                    Err(e) => format!("error: {e}"),
                }
            ));
        }
        drop(device);
        t("gpu: device destroyed");

        // The GPU renderer on the Vulkan backend
        hints::set(hints::GPU_DRIVER, "vulkan").unwrap();
        let r = Renderer::for_window(&g, Some("gpu"));
        t(format!("gpu renderer: {}", result(&r)));
        if let Ok(mut r) = r {
            ev::pump_for(300);
            ev::take();
            let (ow, oh) = r.output_size().unwrap_or_default();
            t(format!(
                "gpu renderer: name={} output={ow}x{oh} vsync={}",
                r.name(),
                r.vsync()
            ));
            scene(&mut r, "gpu clear and fill", Some((0, 0)), &clear_and_fill);
            drop(r);
            t("gpu renderer destroyed");
        }
        hints::reset(hints::GPU_DRIVER);
        g.destroy();
        ev::pump_for(200);
        ev::take();
    }

    pub(super) fn main() {
        t(format!("init: {}", result(&init::init(InitFlags::VIDEO))));
        ev::watch();
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap_or("(none)")
        ));
        surfaces();
        renderer();
        gpu();
        after_opengl();
        ev::unwatch();
        init::quit();
        t("quit");

        let mismatches = MISMATCHES.lock().unwrap().clone();
        for m in &mismatches {
            println!("android_vulkan: against the software renderer: {m}");
        }
        let trace = TRACE.lock().unwrap().clone();
        check_trace(&trace, EXPECTED);
        assert!(
            mismatches.is_empty(),
            "the renderer's pixels differ from the software renderer's: {mismatches:#?}"
        );
    }
}
