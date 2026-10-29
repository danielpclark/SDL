// Tests of Vulkan on the UIKit video driver, through MoltenVK: loading the
// library, its instance extensions, surfaces on its windows, the Vulkan
// renderer there, and the GPU API's Vulkan backend.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver only starts in an app, so this is a program of
//! its own (`harness = false`), run as an app on the iOS Simulator by
//! `tools/ios_run.py` (the `__DATA,__sdl3_runner` section below), as the
//! other `uikit_*` programs are. Its word `moltenvk` asks the runner to
//! bundle MoltenVK's framework for the simulator in the app, as
//! `Frameworks/MoltenVK.framework` (from `SDL3_IOS_MOLTENVK`, see
//! `tools/ios_run.py`); the runner tells the app whether it did in
//! `SDL3_IOS_RUNNER_MOLTENVK` (`1`, or why not: then the program skips,
//! capability `vulkan`, unless `SDL3_TEST_REQUIRE` names it). Its main
//! function traces:
//!
//! * the library: the instance extensions the driver asks for, the
//!   default library (`libvulkan.dylib`, which the app hasn't) and one
//!   that isn't there failing to load, as does a Vulkan window then; the
//!   bundled MoltenVK loaded through `SDL_HINT_VULKAN_LIBRARY`, its
//!   `vkGetInstanceProcAddr`, the loads counted (the same path again, but
//!   not another one) and unloaded;
//! * a Vulkan instance made with those extensions (and one without them):
//!   its physical devices, their queue families;
//! * window V (Vulkan, resizable, high pixel density): a surface on it
//!   (on its Metal view, which becomes the window's view), refused for no
//!   instance and for the instance without the extensions;
//!   `SDL_Vulkan_GetPresentationSupport()` (the driver has no such check:
//!   always supported) and Vulkan's own answer, the surface's
//!   capabilities (its extent, the drawable's size); the surface
//!   destroyed;
//! * the Vulkan renderer on window R (resizable, high pixel density): its
//!   name, output size, maximum texture size, texture formats, vsync and
//!   Vulkan properties, the scenes the other renderers' tests draw
//!   (`uikit_support::scenes`, each read back and hashed, and compared
//!   with the software renderer), vsync 1, 0, 2 and -1, the output
//!   following the window as it turns (iOS);
//! * the GPU API's Vulkan backend: a device, window G claimed (its
//!   swapchain's format, present modes and compositions), frames cleared
//!   and presented (their swapchain textures' sizes, following the window
//!   as it turns), and the "gpu" renderer on the backend.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an iPhone 16 with iOS 18.5),
//! with the same MoltenVK (the release the CI downloads): the hashes are
//! of the same pixels. The mice's device events are left out.
//!
//! It runs on the tvOS Simulator too (an Apple TV 4K (3rd generation) with
//! tvOS 18.5), where nothing turns, and on the visionOS Simulator (an
//! Apple Vision Pro with visionOS 2.5), each against upstream's C there.
//! Elsewhere this program does nothing.

#[cfg(not(any(target_os = "ios", target_os = "tvos", target_os = "visionos")))]
fn main() {}

/// The runner's words: run as an app, with MoltenVK bundled.
#[cfg(any(target_os = "ios", target_os = "tvos", target_os = "visionos"))]
#[used]
#[link_section = "__DATA,__sdl3_runner"]
static RUNNER: [u8; 13] = *b"app moltenvk\0";

#[cfg(any(target_os = "ios", target_os = "tvos", target_os = "visionos"))]
mod uikit_support;

#[cfg(any(target_os = "ios", target_os = "tvos", target_os = "visionos"))]
fn main() {
    ios::main()
}

#[cfg(any(target_os = "ios", target_os = "tvos", target_os = "visionos"))]
mod ios {
    use std::ffi::{c_char, c_void, CStr};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, WindowID};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::render::Renderer;
    use sdl3::video::vulkan;
    use sdl3::video::{self, FColor, Rect, Window};

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::scenes::{
        check_custom_blend, clear_and_fill, hash_read, scene, scenes, t, MISMATCHES, RH, RW, TRACE,
    };
    use super::uikit_support::{check_trace, checked, elapsed, event_name, exit};

    /// Upstream's trace (the C program, on an iPhone 16 simulator with
    /// iOS 18.5).
    #[cfg(target_os = "ios")]
    const EXPECTED: &str = "\
init video: ok
video driver: uikit
application state: 0
instance extensions: VK_KHR_surface, VK_EXT_metal_surface
vkGetInstanceProcAddr unloaded: error: No Vulkan loader has been loaded
load the default library: error: Failed to load Vulkan Portability library
create a vulkan window then: error: Failed to load Vulkan Portability library
load libno-such-vulkan.dylib: error: Failed to load Vulkan Portability library
load MoltenVK (the hint): ok
vkGetInstanceProcAddr: yes
load again: ok
load the same path: ok
load libno-such-vulkan.dylib while loaded: error: Vulkan loader library already loaded
unloaded 1: vkGetInstanceProcAddr: yes
unloaded 2: vkGetInstanceProcAddr: yes
unloaded 3: vkGetInstanceProcAddr: error: No Vulkan loader has been loaded
load MoltenVK again: ok
instance: ok
instance without extensions: ok
physical devices: 1
physical device 0: api=1.0 type=1 name=Apple iOS simulator GPU
physical device 0: queue families: 0x7x1, 0x7x1, 0x7x1, 0x7x1
physical device 0: features: independentBlend=1 imageCubeArray=0 depthClamp=1 shaderClipDistance=1 drawIndirectFirstInstance=0 sampleRateShading=1 samplerAnisotropy=1
V: create 320x480 vulkan resizable high-density: ok
V: created: events: V:WINDOW_SAFE_AREA_CHANGED(0,0), V:WINDOW_MOUSE_ENTER(0,0), V:WINDOW_FOCUS_GAINED(0,0), V:WINDOW_SHOWN(0,0), V:WINDOW_PIXEL_SIZE_CHANGED(1179,2556), V:WINDOW_DISPLAY_SCALE_CHANGED(0,0)
V: created: size=393x852 pixels=1179x2556 density=3 flags=0x10002620 metal_view_tag=255
V: created: root view: SDL_uikitview tag=0 frame=0,0 393x852 layer=CALayer
V: surface without an instance: error: Parameter 'instance' is invalid
V: surface on the instance without extensions: error: VK_EXT_metal_surface or VK_MVK_ios_surface extensions are not enabled in the Vulkan instance.
V: refused: events: (none)
V: refused: size=393x852 pixels=1179x2556 density=3 flags=0x10002620 metal_view_tag=255
V: refused: root view: SDL_uikitview tag=0 frame=0,0 393x852 layer=CALayer
V: surface: ok, a handle
V: surface: events: V:WINDOW_SAFE_AREA_CHANGED(0,0)
V: surface: size=393x852 pixels=1179x2556 density=3 flags=0x10002620 metal_view_tag=255
V: surface: root view: NSKVONotifying_SDL_uikitmetalview tag=255 frame=0,0 393x852 layer=CAMetalLayer drawable=1179x2556
V: physical device 0: presentation support: yes
V: physical device 0: surface support: VkResult 0, 1
V: physical device 0: surface capabilities: VkResult 0, images 2..3 extent 1179x2556 (1x1..8192x8192) layers 1 transforms 0x1 current 0x1 composite alpha 0xd usage 0x1f
presentation support without a physical device: error: Parameter 'physicalDevice' is invalid
V: surface destroyed
V: surface destroyed: events: (none)
V: surface destroyed: size=393x852 pixels=1179x2556 density=3 flags=0x10002620 metal_view_tag=255
V: surface destroyed: root view: SDL_uikitmetalview tag=255 frame=0,0 393x852 layer=CAMetalLayer drawable=1179x2556
V: destroyed
V: destroyed: events: V:WINDOW_DESTROYED(0,0)
instances destroyed
unloaded: vkGetInstanceProcAddr: error: No Vulkan loader has been loaded
R: create 320x480 resizable high-density: ok
R: created: events: R:WINDOW_SAFE_AREA_CHANGED(0,0), R:WINDOW_MOUSE_ENTER(0,0), R:WINDOW_FOCUS_GAINED(0,0), R:WINDOW_SHOWN(0,0), R:WINDOW_PIXEL_SIZE_CHANGED(1179,2556), R:WINDOW_DISPLAY_SCALE_CHANGED(0,0)
R: created: size=393x852 pixels=1179x2556 density=3 flags=0x2620 metal_view_tag=255
R: created: root view: SDL_uikitview tag=0 frame=0,0 393x852 layer=CALayer
R: create vulkan renderer: ok
R: renderer: events: R:WINDOW_HIDDEN(0,0), R:WINDOW_FOCUS_GAINED(0,0), R:WINDOW_SHOWN(0,0), R:WINDOW_SAFE_AREA_CHANGED(0,0)
R: renderer: name=vulkan output=1179x2556 pixels=1179x2556 max_texture_size=16384 vsync=0
R: renderer: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_ABGR2101010, SDL_PIXELFORMAT_RGBA64_FLOAT, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21, SDL_PIXELFORMAT_P010, SDL_PIXELFORMAT_I0FL, SDL_PIXELFORMAT_I4FL
R: renderer: vulkan: instance=yes surface=yes physical device=yes device=yes graphics queue family=0 present queue family=0 swapchain images=3
R: renderer: size=393x852 pixels=1179x2556 density=3 flags=0x10002220 metal_view_tag=255
R: renderer: root view: NSKVONotifying_SDL_uikitmetalview tag=255 frame=0,0 393x852 layer=CAMetalLayer drawable=1179x2556
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
scene linear scaling: 64x48 11a331c0
scene debug text: 64x48 29a1d304
partial read: 8x8 f24b7925
target read: 20x10 da6561f5
R: vsync 1: ok, vsync=1
R: vsync 0: ok, vsync=0
R: vsync 2: error: That operation is not supported, vsync=0
R: vsync -1: ok, vsync=-1
R: scenes: events: (none)
R: renderer landscape right: events: DISPLAY_DESKTOP_MODE_CHANGED(1,852), DISPLAY_CURRENT_MODE_CHANGED(1,852), DISPLAY_ORIENTATION(1,1), R:WINDOW_METAL_VIEW_RESIZED(0,0), R:WINDOW_RESIZED(852,393), R:WINDOW_PIXEL_SIZE_CHANGED(2556,1179), R:WINDOW_SAFE_AREA_CHANGED(0,0)
R: renderer landscape right presented: events: (none)
R: renderer landscape right: name=vulkan output=2556x1179 pixels=2556x1179 max_texture_size=16384 vsync=-1
R: renderer landscape right: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_ABGR2101010, SDL_PIXELFORMAT_RGBA64_FLOAT, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21, SDL_PIXELFORMAT_P010, SDL_PIXELFORMAT_I0FL, SDL_PIXELFORMAT_I4FL
R: renderer landscape right: vulkan: instance=yes surface=yes physical device=yes device=yes graphics queue family=0 present queue family=0 swapchain images=3
R: renderer landscape right: size=852x393 pixels=2556x1179 density=3 flags=0x10002220 metal_view_tag=255
R: renderer landscape right: root view: NSKVONotifying_SDL_uikitmetalview tag=255 frame=0,0 852x393 layer=CAMetalLayer drawable=2556x1179
R: renderer landscape right corner: 64x48 bf597dc5
R: renderer portrait: events: DISPLAY_DESKTOP_MODE_CHANGED(1,393), DISPLAY_CURRENT_MODE_CHANGED(1,393), DISPLAY_ORIENTATION(1,3), R:WINDOW_METAL_VIEW_RESIZED(0,0), R:WINDOW_RESIZED(393,852), R:WINDOW_PIXEL_SIZE_CHANGED(1179,2556), R:WINDOW_SAFE_AREA_CHANGED(0,0)
R: renderer portrait presented: events: (none)
R: renderer portrait: name=vulkan output=1179x2556 pixels=1179x2556 max_texture_size=16384 vsync=-1
R: renderer portrait: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_ABGR2101010, SDL_PIXELFORMAT_RGBA64_FLOAT, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21, SDL_PIXELFORMAT_P010, SDL_PIXELFORMAT_I0FL, SDL_PIXELFORMAT_I4FL
R: renderer portrait: vulkan: instance=yes surface=yes physical device=yes device=yes graphics queue family=0 present queue family=0 swapchain images=3
R: renderer portrait: size=393x852 pixels=1179x2556 density=3 flags=0x10002220 metal_view_tag=255
R: renderer portrait: root view: NSKVONotifying_SDL_uikitmetalview tag=255 frame=0,0 393x852 layer=CAMetalLayer drawable=1179x2556
scene again clear and fill: 64x48 5fe1f24a
R: renderer destroyed
R: renderer destroyed: events: (none)
R: renderer destroyed: size=393x852 pixels=1179x2556 density=3 flags=0x10002220 metal_view_tag=255
R: renderer destroyed: root view: SDL_uikitmetalview tag=255 frame=0,0 393x852 layer=CAMetalLayer drawable=1179x2556
R: destroyed
R: destroyed: events: R:WINDOW_DESTROYED(0,0)
gpu: supports SPIR-V on vulkan: no
gpu: create vulkan device: error: SDL_HINT_GPU_DRIVER vulkan unsupported!
quit
";

    /// Upstream's trace on tvOS (the C program, on an Apple TV 4K (3rd
    /// generation) simulator with tvOS 18.5).
    #[cfg(target_os = "tvos")]
    const EXPECTED: &str = "\
init video: ok
video driver: uikit
application state: 0
instance extensions: VK_KHR_surface, VK_EXT_metal_surface
vkGetInstanceProcAddr unloaded: error: No Vulkan loader has been loaded
load the default library: error: Failed to load Vulkan Portability library
create a vulkan window then: error: Failed to load Vulkan Portability library
load libno-such-vulkan.dylib: error: Failed to load Vulkan Portability library
load MoltenVK (the hint): ok
vkGetInstanceProcAddr: yes
load again: ok
load the same path: ok
load libno-such-vulkan.dylib while loaded: error: Vulkan loader library already loaded
unloaded 1: vkGetInstanceProcAddr: yes
unloaded 2: vkGetInstanceProcAddr: yes
unloaded 3: vkGetInstanceProcAddr: error: No Vulkan loader has been loaded
load MoltenVK again: ok
instance: ok
instance without extensions: ok
physical devices: 1
physical device 0: api=1.0 type=1 name=Apple tvOS simulator GPU
physical device 0: queue families: 0x7x1, 0x7x1, 0x7x1, 0x7x1
physical device 0: features: independentBlend=1 imageCubeArray=0 depthClamp=1 shaderClipDistance=1 drawIndirectFirstInstance=0 sampleRateShading=1 samplerAnisotropy=1
V: create 320x480 vulkan resizable high-density: ok
V: created: events: V:WINDOW_SAFE_AREA_CHANGED(0,0), V:WINDOW_MOUSE_ENTER(0,0), V:WINDOW_FOCUS_GAINED(0,0), V:WINDOW_SHOWN(0,0), V:WINDOW_PIXEL_SIZE_CHANGED(3840,2160), V:WINDOW_DISPLAY_SCALE_CHANGED(0,0)
V: created: size=1920x1080 pixels=3840x2160 density=2 flags=0x10002620 metal_view_tag=255
V: created: root view: SDL_uikitview tag=0 frame=0,0 1920x1080 layer=CALayer
V: surface without an instance: error: Parameter 'instance' is invalid
V: surface on the instance without extensions: error: VK_EXT_metal_surface or VK_MVK_ios_surface extensions are not enabled in the Vulkan instance.
V: refused: events: (none)
V: refused: size=1920x1080 pixels=3840x2160 density=2 flags=0x10002620 metal_view_tag=255
V: refused: root view: SDL_uikitview tag=0 frame=0,0 1920x1080 layer=CALayer
V: surface: ok, a handle
V: surface: events: (none)
V: surface: size=1920x1080 pixels=3840x2160 density=2 flags=0x10002620 metal_view_tag=255
V: surface: root view: NSKVONotifying_SDL_uikitmetalview tag=255 frame=0,0 1920x1080 layer=CAMetalLayer drawable=3840x2160
V: physical device 0: presentation support: yes
V: physical device 0: surface support: VkResult 0, 1
V: physical device 0: surface capabilities: VkResult 0, images 2..3 extent 3840x2160 (1x1..8192x8192) layers 1 transforms 0x1 current 0x1 composite alpha 0xd usage 0x1f
presentation support without a physical device: error: Parameter 'physicalDevice' is invalid
V: surface destroyed
V: surface destroyed: events: (none)
V: surface destroyed: size=1920x1080 pixels=3840x2160 density=2 flags=0x10002620 metal_view_tag=255
V: surface destroyed: root view: SDL_uikitmetalview tag=255 frame=0,0 1920x1080 layer=CAMetalLayer drawable=3840x2160
V: destroyed
V: destroyed: events: V:WINDOW_DESTROYED(0,0)
instances destroyed
unloaded: vkGetInstanceProcAddr: error: No Vulkan loader has been loaded
R: create 320x480 resizable high-density: ok
R: created: events: R:WINDOW_SAFE_AREA_CHANGED(0,0), R:WINDOW_MOUSE_ENTER(0,0), R:WINDOW_FOCUS_GAINED(0,0), R:WINDOW_SHOWN(0,0), R:WINDOW_PIXEL_SIZE_CHANGED(3840,2160), R:WINDOW_DISPLAY_SCALE_CHANGED(0,0)
R: created: size=1920x1080 pixels=3840x2160 density=2 flags=0x2620 metal_view_tag=255
R: created: root view: SDL_uikitview tag=0 frame=0,0 1920x1080 layer=CALayer
R: create vulkan renderer: ok
R: renderer: events: R:WINDOW_HIDDEN(0,0), R:WINDOW_FOCUS_GAINED(0,0), R:WINDOW_SHOWN(0,0)
R: renderer: name=vulkan output=3840x2160 pixels=3840x2160 max_texture_size=16384 vsync=0
R: renderer: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_ABGR2101010, SDL_PIXELFORMAT_RGBA64_FLOAT, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21, SDL_PIXELFORMAT_P010, SDL_PIXELFORMAT_I0FL, SDL_PIXELFORMAT_I4FL
R: renderer: vulkan: instance=yes surface=yes physical device=yes device=yes graphics queue family=0 present queue family=0 swapchain images=3
R: renderer: size=1920x1080 pixels=3840x2160 density=2 flags=0x10002220 metal_view_tag=255
R: renderer: root view: NSKVONotifying_SDL_uikitmetalview tag=255 frame=0,0 1920x1080 layer=CAMetalLayer drawable=3840x2160
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
scene linear scaling: 64x48 11a331c0
scene debug text: 64x48 29a1d304
partial read: 8x8 f24b7925
target read: 20x10 da6561f5
R: vsync 1: ok, vsync=1
R: vsync 0: ok, vsync=0
R: vsync 2: error: That operation is not supported, vsync=0
R: vsync -1: ok, vsync=-1
R: scenes: events: (none)
R: renderer landscape right: not turned (no orientations on tvOS)
R: renderer landscape right: events: (none)
R: renderer landscape right presented: events: (none)
R: renderer landscape right: name=vulkan output=3840x2160 pixels=3840x2160 max_texture_size=16384 vsync=-1
R: renderer landscape right: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_ABGR2101010, SDL_PIXELFORMAT_RGBA64_FLOAT, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21, SDL_PIXELFORMAT_P010, SDL_PIXELFORMAT_I0FL, SDL_PIXELFORMAT_I4FL
R: renderer landscape right: vulkan: instance=yes surface=yes physical device=yes device=yes graphics queue family=0 present queue family=0 swapchain images=3
R: renderer landscape right: size=1920x1080 pixels=3840x2160 density=2 flags=0x10002220 metal_view_tag=255
R: renderer landscape right: root view: NSKVONotifying_SDL_uikitmetalview tag=255 frame=0,0 1920x1080 layer=CAMetalLayer drawable=3840x2160
R: renderer landscape right corner: 64x48 bf597dc5
R: renderer portrait: not turned (no orientations on tvOS)
R: renderer portrait: events: (none)
R: renderer portrait presented: events: (none)
R: renderer portrait: name=vulkan output=3840x2160 pixels=3840x2160 max_texture_size=16384 vsync=-1
R: renderer portrait: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_ABGR2101010, SDL_PIXELFORMAT_RGBA64_FLOAT, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21, SDL_PIXELFORMAT_P010, SDL_PIXELFORMAT_I0FL, SDL_PIXELFORMAT_I4FL
R: renderer portrait: vulkan: instance=yes surface=yes physical device=yes device=yes graphics queue family=0 present queue family=0 swapchain images=3
R: renderer portrait: size=1920x1080 pixels=3840x2160 density=2 flags=0x10002220 metal_view_tag=255
R: renderer portrait: root view: NSKVONotifying_SDL_uikitmetalview tag=255 frame=0,0 1920x1080 layer=CAMetalLayer drawable=3840x2160
scene again clear and fill: 64x48 5fe1f24a
R: renderer destroyed
R: renderer destroyed: events: (none)
R: renderer destroyed: size=1920x1080 pixels=3840x2160 density=2 flags=0x10002220 metal_view_tag=255
R: renderer destroyed: root view: SDL_uikitmetalview tag=255 frame=0,0 1920x1080 layer=CAMetalLayer drawable=3840x2160
R: destroyed
R: destroyed: events: R:WINDOW_DESTROYED(0,0)
gpu: supports SPIR-V on vulkan: no
gpu: create vulkan device: error: SDL_HINT_GPU_DRIVER vulkan unsupported!
quit
";

    /// Upstream's trace on visionOS (the C program, on an Apple Vision Pro
    /// simulator with visionOS 2.5).
    #[cfg(target_os = "visionos")]
    const EXPECTED: &str = "\
init video: ok
video driver: uikit
application state: 0
instance extensions: VK_KHR_surface, VK_EXT_metal_surface
vkGetInstanceProcAddr unloaded: error: No Vulkan loader has been loaded
load the default library: error: Failed to load Vulkan Portability library
create a vulkan window then: error: Failed to load Vulkan Portability library
load libno-such-vulkan.dylib: error: Failed to load Vulkan Portability library
load MoltenVK (the hint): error: Failed to load Vulkan Portability library
vkGetInstanceProcAddr: error: No Vulkan loader has been loaded
load again: error: Failed to load Vulkan Portability library
load the same path: error: Failed to load Vulkan Portability library
load libno-such-vulkan.dylib while loaded: error: Failed to load Vulkan Portability library
unloaded 1: vkGetInstanceProcAddr: error: No Vulkan loader has been loaded
unloaded 2: vkGetInstanceProcAddr: error: No Vulkan loader has been loaded
unloaded 3: vkGetInstanceProcAddr: error: No Vulkan loader has been loaded
load MoltenVK again: error: Failed to load Vulkan Portability library
quit
";

    /// Where the runner bundles MoltenVK (`SDL_HINT_VULKAN_LIBRARY`'s
    /// value here).
    const MOLTENVK: &str = "@executable_path/Frameworks/MoltenVK.framework/MoltenVK";

    /// A library that isn't there.
    const NO_SUCH_LIBRARY: &str = "libno-such-vulkan.dylib";

    fn yes_no(b: bool) -> &'static str {
        if b {
            "yes"
        } else {
            "no"
        }
    }

    fn list(items: Vec<String>) -> String {
        if items.is_empty() {
            "(none)".into()
        } else {
            items.join(", ")
        }
    }

    fn ok<T>(r: &sdl3::Result<T>) -> String {
        match r {
            Ok(_) => "ok".into(),
            Err(e) => format!("error: {e}"),
        }
    }

    /// The windows the program made, by ID, for the events' labels.
    static LABELS: Mutex<Vec<(WindowID, &'static str)>> = Mutex::new(Vec::new());

    fn name_window(w: &Window, label: &'static str) {
        LABELS.lock().unwrap().push((w.id(), label));
    }

    fn window_label(id: WindowID) -> &'static str {
        LABELS
            .lock()
            .unwrap()
            .iter()
            .find(|(i, _)| *i == id)
            .map_or("?", |(_, l)| l)
    }

    /// An event as the trace names it (the C program's names), `None` for
    /// the mice's device events (see the module's description).
    fn describe(event: &Event) -> Option<String> {
        let name = event_name(event.event_type());
        Some(match event {
            Event::MouseDevice(_) => return None,
            Event::Display(d) => format!("{name}({},{})", d.display_id, d.data1),
            Event::Window(w) => format!(
                "{}:{name}({},{})",
                window_label(w.window_id),
                w.data1,
                w.data2
            ),
            _ => name,
        })
    }

    /// Pump, and the queued events.
    fn take_queued(out: &mut Vec<String>) {
        events::pump();
        while let Some(event) = events::poll() {
            out.extend(describe(&event));
        }
    }

    /// Pump for `ms` milliseconds (10 ms steps).
    fn settle(out: &mut Vec<String>, ms: u64) {
        take_queued(out);
        for _ in 0..ms / 10 {
            std::thread::sleep(Duration::from_millis(10));
            take_queued(out);
        }
    }

    /// Pump until an event containing `until` has come (then for
    /// `after_ms` more), or for at most `limit`.
    fn pump_until(out: &mut Vec<String>, until: &str, limit: Duration, after_ms: u64) {
        let end = Instant::now() + limit;
        while Instant::now() < end {
            take_queued(out);
            if out.iter().any(|e| e.contains(until)) {
                settle(out, after_ms);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn trace_events(label: &str, ms: u64) {
        let mut seen = Vec::new();
        settle(&mut seen, ms);
        t(format!("{label}: events: {}", list(seen)));
    }

    /// `CGRect`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }

    /// `CGSize`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGSize {
        w: f64,
        h: f64,
    }

    fn rect(r: CGRect) -> String {
        format!("{},{} {}x{}", r.x, r.y, r.w, r.h)
    }

    /// The window's `UIWindow` (its property, a number here).
    fn uiwindow_of(w: &Window) -> Id {
        w.properties()
            .ok()
            .and_then(|p| p.get_number(video::window::PROP_WINDOW_UIKIT_WINDOW_POINTER))
            .map_or(std::ptr::null_mut(), |p| p as usize as Id)
    }

    /// Its root view controller.
    fn vc_of(w: &Window) -> Id {
        // SAFETY: a UIWindow property.
        unsafe { objc::send0(uiwindow_of(w), "rootViewController") }
    }

    /// Its root view controller's view.
    fn root_view_of(w: &Window) -> Id {
        // SAFETY: a UIViewController property.
        unsafe { objc::send0(vc_of(w), "view") }
    }

    /// A view's frame.
    fn frame_of(view: Id) -> CGRect {
        if view.is_null() {
            return CGRect::default();
        }
        // SAFETY: a UIView property.
        unsafe { objc::send0(view, "frame") }
    }

    /// The window's sizes and flags, its Metal view tag and its view
    /// controller's view (and that view's layer).
    fn window_state(label: &str, w: &Window) {
        let (ww, wh) = w.size().unwrap_or_default();
        let (pw, ph) = w.size_in_pixels().unwrap_or_default();
        // (without the OpenGL flag, which a window that asks for no
        // graphics API gets on UIKit, as the C program traces them)
        let flags = w.flags().unwrap_or_default().0 & !WindowFlags::OPENGL.0;
        let tag = w
            .properties()
            .ok()
            .and_then(|p| p.get_number(video::window::PROP_WINDOW_UIKIT_METAL_VIEW_TAG_NUMBER))
            .unwrap_or(-1);
        t(format!(
            "{label}: size={ww}x{wh} pixels={pw}x{ph} density={} flags={flags:#x} metal_view_tag={tag}",
            w.pixel_density().unwrap_or_default()
        ));
        let v = root_view_of(w);
        if v.is_null() {
            t(format!("{label}: root view: nil"));
            return;
        }
        // SAFETY: UIView's and CALayer's properties (a CAMetalLayer's
        // drawable size when it is one).
        unsafe {
            let view_tag: isize = objc::send0(v, "tag");
            let layer: Id = objc::send0(v, "layer");
            let layer_class = objc::class_name(layer);
            let drawable = if layer_class == "CAMetalLayer" {
                let size: CGSize = objc::send0(layer, "drawableSize");
                format!(" drawable={}x{}", size.w, size.h)
            } else {
                String::new()
            };
            t(format!(
                "{label}: root view: {} tag={view_tag} frame={} layer={layer_class}{drawable}",
                objc::class_name(v),
                rect(frame_of(v))
            ));
        }
    }

    /// Ask the window's scene to turn to an orientation (a
    /// `UIInterfaceOrientationMask`), and trace the events that come.
    fn turn(w: &Window, label: &str, mask: usize) {
        if cfg!(any(target_os = "tvos", target_os = "visionos")) {
            let _ = (w, mask);
            t(format!(
                "{label}: not turned (no orientations on {})",
                if cfg!(target_os = "tvos") {
                    "tvOS"
                } else {
                    "visionOS"
                }
            ));
            trace_events(label, 500);
            return;
        }
        let mut seen = Vec::new();
        println!("uikit_vulkan: {:.3} s: turning to {label}", elapsed());
        // SAFETY: UIWindowSceneGeometryPreferencesIOS's initializer, the
        // view controller's and the scene's methods (iOS 16).
        unsafe {
            let prefs: Id =
                objc::send0(objc::class("UIWindowSceneGeometryPreferencesIOS"), "alloc");
            let prefs: Id = objc::send1(prefs, "initWithInterfaceOrientations:", mask);
            let () = objc::send0(vc_of(w), "setNeedsUpdateOfSupportedInterfaceOrientations");
            let scene: Id = objc::send0(uiwindow_of(w), "windowScene");
            let () = objc::send2(
                scene,
                "requestGeometryUpdateWithPreferences:errorHandler:",
                prefs,
                std::ptr::null_mut::<c_void>(),
            );
        }
        pump_until(
            &mut seen,
            "DISPLAY_ORIENTATION",
            Duration::from_secs(5),
            1500,
        );
        println!("uikit_vulkan: {:.3} s: turned", elapsed());
        t(format!("{label}: events: {}", list(seen)));
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
            let info = VkInstanceCreateInfo {
                s_type: 1, // VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO
                p_next: std::ptr::null(),
                flags: 0,
                p_application_info: std::ptr::null(),
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

        fn physical_devices(&self, instance: VkInstance) -> Vec<VkPhysicalDevice> {
            type Enumerate =
                unsafe extern "system" fn(VkInstance, *mut u32, *mut VkPhysicalDevice) -> i32;
            // SAFETY: the instance's function, with room for the count.
            unsafe {
                let enumerate: Enumerate = self.f(instance, c"vkEnumeratePhysicalDevices");
                let mut count = 0;
                enumerate(instance, &mut count, std::ptr::null_mut());
                let mut devices = vec![std::ptr::null_mut(); count as usize];
                enumerate(instance, &mut count, devices.as_mut_ptr());
                devices.truncate(count as usize);
                devices
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
        let formats: Vec<String> = r
            .texture_formats()
            .iter()
            .map(|f| f.name().to_string())
            .collect();
        t(format!("{label}: formats: {}", list(formats)));
        let set = |name: &str| yes_no(props.get_number(name).unwrap_or(0) != 0);
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
        window_state(label, w);
    }

    fn application_state() -> isize {
        // SAFETY: a UIApplication property.
        unsafe { objc::send0(objc::shared_application(), "applicationState") }
    }

    pub(super) fn main() {
        elapsed();
        let status = sdl3::app::run_app(|_args| {
            let status = checked(main_function);
            exit(status);
        });
        println!("uikit_vulkan: run_app returned {status}, which it never does");
        exit(1);
    }

    /// Whether the runner bundled MoltenVK (else the program skips, unless
    /// `SDL3_TEST_REQUIRE` names `vulkan`).
    fn moltenvk_bundled() -> bool {
        let why = match std::env::var("SDL3_IOS_RUNNER_MOLTENVK") {
            Ok(v) if v == "1" => return true,
            Ok(v) => v,
            Err(_) => "the runner doesn't bundle MoltenVK".to_string(),
        };
        let required = std::env::var("SDL3_TEST_REQUIRE").unwrap_or_default();
        if required
            .split(',')
            .map(str::trim)
            .any(|c| c == "vulkan" || c == "all")
        {
            panic!("required capability vulkan unavailable: {why}");
        }
        eprintln!("note: skipping, vulkan unavailable: {why}");
        false
    }

    fn main_function() {
        const LANDSCAPE_RIGHT: usize = 1 << 3;
        const PORTRAIT: usize = 1 << 1;

        println!("uikit_vulkan: {:.3} s: main", elapsed());
        if !moltenvk_bundled() {
            return;
        }
        let r = init::init(InitFlags::VIDEO);
        t(format!("init video: {}", ok(&r)));
        r.unwrap();
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap()
        ));

        // The app becomes active around when the main function starts
        // (the events of its activation are left out: they may come before
        // SDL is up)
        let mut ignored = Vec::new();
        let end = Instant::now() + Duration::from_secs(30);
        while application_state() != 0 && Instant::now() < end {
            take_queued(&mut ignored);
            std::thread::sleep(Duration::from_millis(10));
        }
        for _ in 0..10 {
            take_queued(&mut ignored);
            std::thread::sleep(Duration::from_millis(10));
        }
        t(format!("application state: {}", application_state()));

        // The library
        t(format!(
            "instance extensions: {}",
            match vulkan::vulkan_instance_extensions() {
                Ok(e) => e.join(", "),
                Err(e) => format!("error: {e}"),
            }
        ));
        t(format!(
            "vkGetInstanceProcAddr unloaded: {}",
            ok(&vulkan::vulkan_get_vk_get_instance_proc_addr())
        ));
        t(format!(
            "load the default library: {}",
            ok(&vulkan::vulkan_load_library(None))
        ));
        let r = Window::create("V", 320, 480, WindowFlags::VULKAN);
        t(format!("create a vulkan window then: {}", ok(&r)));
        if let Ok(w) = r {
            w.destroy();
        }
        t(format!(
            "load {NO_SUCH_LIBRARY}: {}",
            ok(&vulkan::vulkan_load_library(Some(NO_SUCH_LIBRARY)))
        ));
        hints::set(hints::VULKAN_LIBRARY, MOLTENVK).unwrap();
        t(format!(
            "load MoltenVK (the hint): {}",
            ok(&vulkan::vulkan_load_library(None))
        ));
        t(format!(
            "vkGetInstanceProcAddr: {}",
            match vulkan::vulkan_get_vk_get_instance_proc_addr() {
                Ok(f) => yes_no(f != 0).to_string(),
                Err(e) => format!("error: {e}"),
            }
        ));
        t(format!(
            "load again: {}",
            ok(&vulkan::vulkan_load_library(None))
        ));
        t(format!(
            "load the same path: {}",
            ok(&vulkan::vulkan_load_library(Some(MOLTENVK)))
        ));
        t(format!(
            "load {NO_SUCH_LIBRARY} while loaded: {}",
            ok(&vulkan::vulkan_load_library(Some(NO_SUCH_LIBRARY)))
        ));
        for i in 1..=3 {
            vulkan::vulkan_unload_library();
            t(format!(
                "unloaded {i}: vkGetInstanceProcAddr: {}",
                match vulkan::vulkan_get_vk_get_instance_proc_addr() {
                    Ok(f) => yes_no(f != 0).to_string(),
                    Err(e) => format!("error: {e}"),
                }
            ));
        }
        let r = vulkan::vulkan_load_library(None);
        t(format!("load MoltenVK again: {}", ok(&r)));
        if let Err(e) = r {
            // (as on visionOS, where upstream's SDL_LoadObject() refuses
            // every library: see the FIXME (upstream) on
            // sdl3::loadso::SharedObject::load)
            let why = if cfg!(target_os = "visionos") {
                " (upstream's SDL_LoadObject() requires iOS 8+, and visionOS's version is below 8)"
            } else {
                ""
            };
            eprintln!("note: skipping, vulkan unavailable: MoltenVK isn't loaded ({e}){why}");
            init::quit();
            t("quit");
            check_trace(&TRACE.lock().unwrap(), EXPECTED);
            println!("uikit_vulkan: all checks passed");
            return;
        }

        // An instance, with the extensions and without them
        let vk = Vk::new();
        let extensions = vulkan::vulkan_instance_extensions().unwrap();
        let instance = vk.create_instance(&extensions);
        t(format!(
            "instance: {}",
            match &instance {
                Ok(_) => "ok".to_string(),
                Err(result) => format!("VkResult {result}"),
            }
        ));
        let instance = instance.unwrap();
        let plain = vk.create_instance(&[]);
        t(format!(
            "instance without extensions: {}",
            match &plain {
                Ok(_) => "ok".to_string(),
                Err(result) => format!("VkResult {result}"),
            }
        ));
        let plain = plain.unwrap();
        let devices = vk.physical_devices(instance);
        t(format!("physical devices: {}", devices.len()));
        for (i, &device) in devices.iter().enumerate() {
            t(format!(
                "physical device {i}: {}",
                vk.device_properties(instance, device)
            ));
            let families: Vec<String> = vk
                .queue_families(instance, device)
                .iter()
                .map(|f| format!("0x{:x}x{}", f.queue_flags, f.queue_count))
                .collect();
            t(format!(
                "physical device {i}: queue families: {}",
                list(families)
            ));
            t(format!(
                "physical device {i}: features: {}",
                vk.gpu_features(instance, device)
            ));
        }

        // V: a Vulkan window and a surface on it
        let r = Window::create(
            "V",
            320,
            480,
            WindowFlags::VULKAN | WindowFlags::RESIZABLE | WindowFlags::HIGH_PIXEL_DENSITY,
        );
        t(format!(
            "V: create 320x480 vulkan resizable high-density: {}",
            ok(&r)
        ));
        let v = r.unwrap();
        name_window(&v, "V");
        trace_events("V: created", 500);
        window_state("V: created", &v);
        t(format!(
            "V: surface without an instance: {}",
            ok(&vulkan::vulkan_create_surface(&v, 0, 0))
        ));
        t(format!(
            "V: surface on the instance without extensions: {}",
            ok(&vulkan::vulkan_create_surface(&v, plain as usize, 0))
        ));
        trace_events("V: refused", 300);
        window_state("V: refused", &v);
        let r = vulkan::vulkan_create_surface(&v, instance as usize, 0);
        t(format!(
            "V: surface: {}",
            match &r {
                Ok(s) => format!("ok, {}", if *s != 0 { "a handle" } else { "null" }),
                Err(e) => format!("error: {e}"),
            }
        ));
        let surface = r.unwrap();
        trace_events("V: surface", 500);
        window_state("V: surface", &v);
        for (i, &device) in devices.iter().enumerate() {
            t(format!(
                "V: physical device {i}: presentation support: {}",
                match vulkan::vulkan_presentation_support(instance as usize, device as usize, 0) {
                    Ok(b) => yes_no(b).to_string(),
                    Err(e) => format!("error: {e}"),
                }
            ));
            let (result, supported) = vk.surface_support(instance, device, 0, surface);
            t(format!(
                "V: physical device {i}: surface support: VkResult {result}, {supported}"
            ));
            let (result, c) = vk.surface_capabilities(instance, device, surface);
            t(format!(
                "V: physical device {i}: surface capabilities: VkResult {result}, images {}..{} extent {}x{} ({}x{}..{}x{}) layers {} transforms 0x{:x} current 0x{:x} composite alpha 0x{:x} usage 0x{:x}",
                c.min_image_count,
                c.max_image_count,
                c.current_extent[0],
                c.current_extent[1],
                c.min_image_extent[0],
                c.min_image_extent[1],
                c.max_image_extent[0],
                c.max_image_extent[1],
                c.max_image_array_layers,
                c.supported_transforms,
                c.current_transform,
                c.supported_composite_alpha,
                c.supported_usage_flags
            ));
        }
        t(format!(
            "presentation support without a physical device: {}",
            ok(&vulkan::vulkan_presentation_support(
                instance as usize,
                0,
                0
            ))
        ));
        vulkan::vulkan_destroy_surface(instance as usize, surface, 0);
        t("V: surface destroyed");
        trace_events("V: surface destroyed", 300);
        window_state("V: surface destroyed", &v);
        v.destroy();
        t("V: destroyed");
        trace_events("V: destroyed", 500);
        vk.destroy_instance(plain);
        vk.destroy_instance(instance);
        t("instances destroyed");
        vulkan::vulkan_unload_library();
        t(format!(
            "unloaded: vkGetInstanceProcAddr: {}",
            ok(&vulkan::vulkan_get_vk_get_instance_proc_addr())
        ));

        // R: the Vulkan renderer
        let r = Window::create(
            "R",
            320,
            480,
            WindowFlags::RESIZABLE | WindowFlags::HIGH_PIXEL_DENSITY,
        );
        t(format!(
            "R: create 320x480 resizable high-density: {}",
            ok(&r)
        ));
        let rw = r.unwrap();
        name_window(&rw, "R");
        trace_events("R: created", 500);
        window_state("R: created", &rw);
        let r = Renderer::for_window(&rw, Some("vulkan"));
        t(format!("R: create vulkan renderer: {}", ok(&r)));
        let mut r = r.unwrap();
        trace_events("R: renderer", 500);
        renderer_state("R: renderer", &mut r, &rw);
        let _ = r.present();
        scenes(&mut r, "Vulkan");
        for want in [1, 0, 2, -1] {
            let result = r.set_vsync(want);
            t(format!(
                "R: vsync {want}: {}, vsync={}",
                ok(&result),
                r.vsync()
            ));
            r.set_draw_color(0, 0, 0, 255);
            let _ = r.clear();
            let _ = r.present();
        }
        trace_events("R: scenes", 300);

        // The output follows the window as it turns
        turn(&rw, "R: renderer landscape right", LANDSCAPE_RIGHT);
        r.set_draw_color(0, 128, 0, 255);
        let _ = r.clear();
        let _ = r.present();
        trace_events("R: renderer landscape right presented", 300);
        renderer_state("R: renderer landscape right", &mut r, &rw);
        r.set_draw_color(0, 128, 0, 255);
        let _ = r.clear();
        let (ow, oh) = r.output_size().unwrap_or_default();
        hash_read(
            "R: renderer landscape right corner",
            r.read_pixels(Some(&Rect::new(ow - RW, oh - RH, RW, RH))),
        );
        let _ = r.present();
        turn(&rw, "R: renderer portrait", PORTRAIT);
        let _ = r.present();
        trace_events("R: renderer portrait presented", 300);
        renderer_state("R: renderer portrait", &mut r, &rw);
        scene(
            &mut r,
            "again clear and fill",
            Some((0, 0)),
            &clear_and_fill,
        );
        check_custom_blend(&mut r);
        drop(r);
        t("R: renderer destroyed");
        trace_events("R: renderer destroyed", 300);
        window_state("R: renderer destroyed", &rw);
        rw.destroy();
        t("R: destroyed");
        trace_events("R: destroyed", 500);

        gpu();

        init::quit();
        t("quit");

        let mismatches = MISMATCHES.lock().unwrap();
        for m in mismatches.iter() {
            println!("uikit_vulkan: against the software renderer: {m}");
        }
        check_trace(&TRACE.lock().unwrap(), EXPECTED);
        assert!(
            mismatches.is_empty(),
            "the Vulkan renderers against the software renderer:\n{}",
            mismatches.join("\n")
        );
        println!("uikit_vulkan: all checks passed");
    }

    /// The GPU API's Vulkan backend on window G.
    fn gpu() {
        use sdl3::gpu::{
            ColorTargetInfo, Device, LoadOp, PresentMode, ShaderFormat, SwapchainComposition,
        };
        const LANDSCAPE_RIGHT: usize = 1 << 3;
        const PORTRAIT: usize = 1 << 1;

        t(format!(
            "gpu: supports SPIR-V on vulkan: {}",
            yes_no(sdl3::gpu::supports_shader_formats(
                ShaderFormat::SPIRV,
                Some("vulkan")
            ))
        ));
        let device = Device::new(ShaderFormat::SPIRV, false, Some("vulkan"));
        t(format!("gpu: create vulkan device: {}", ok(&device)));
        let Ok(device) = device else {
            return;
        };
        t(format!(
            "gpu: driver={} shader formats=0x{:x}",
            device.driver(),
            device.shader_formats().0
        ));
        let r = Window::create(
            "G",
            320,
            480,
            WindowFlags::RESIZABLE | WindowFlags::HIGH_PIXEL_DENSITY,
        );
        t(format!(
            "G: create 320x480 resizable high-density: {}",
            ok(&r)
        ));
        let g = r.unwrap();
        name_window(&g, "G");
        trace_events("G: created", 500);
        let r = device.claim_window(&g);
        t(format!("G: claim: {}", ok(&r)));
        if r.is_err() {
            drop(device);
            g.destroy();
            t("G: destroyed");
            return;
        }
        trace_events("G: claimed", 500);
        window_state("G: claimed", &g);
        t(format!(
            "G: swapchain format: {}",
            match device.swapchain_texture_format(&g) {
                Ok(f) => f.0.to_string(),
                Err(e) => format!("error: {e}"),
            }
        ));
        t(format!(
            "G: present modes: vsync={} immediate={} mailbox={}",
            yes_no(device.window_supports_present_mode(&g, PresentMode::Vsync)),
            yes_no(device.window_supports_present_mode(&g, PresentMode::Immediate)),
            yes_no(device.window_supports_present_mode(&g, PresentMode::Mailbox))
        ));
        t(format!(
            "G: compositions: sdr={} sdr linear={} hdr extended linear={} hdr10={}",
            yes_no(device.window_supports_swapchain_composition(&g, SwapchainComposition::Sdr)),
            yes_no(
                device.window_supports_swapchain_composition(&g, SwapchainComposition::SdrLinear)
            ),
            yes_no(device.window_supports_swapchain_composition(
                &g,
                SwapchainComposition::HdrExtendedLinear
            )),
            yes_no(
                device.window_supports_swapchain_composition(&g, SwapchainComposition::Hdr10St2084)
            )
        ));
        let blue = FColor::new(0.0, 0.0, 1.0, 1.0);
        let frame = |label: &str| {
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
            t(format!("{label}: {what}, submitted: {}", ok(&submitted)));
        };
        for i in 1..=3 {
            frame(&format!("G: frame {i}"));
        }
        turn(&g, "G: landscape right", LANDSCAPE_RIGHT);
        for i in 1..=2 {
            frame(&format!("G: landscape right frame {i}"));
        }
        turn(&g, "G: portrait", PORTRAIT);
        for i in 1..=2 {
            frame(&format!("G: portrait frame {i}"));
        }
        t(format!("G: wait for idle: {}", ok(&device.wait_for_idle())));
        device.release_window(&g);
        t(format!(
            "G: released: swapchain format: {}",
            match device.swapchain_texture_format(&g) {
                Ok(f) => f.0.to_string(),
                Err(e) => format!("error: {e}"),
            }
        ));
        trace_events("G: released", 300);
        drop(device);
        t("gpu: device destroyed");

        // The GPU renderer on the Vulkan backend
        hints::set(hints::GPU_DRIVER, "vulkan").unwrap();
        let r = Renderer::for_window(&g, Some("gpu"));
        t(format!("G: create gpu renderer: {}", ok(&r)));
        if let Ok(mut r) = r {
            trace_events("G: gpu renderer", 500);
            let (ow, oh) = r.output_size().unwrap_or_default();
            t(format!(
                "G: gpu renderer: name={} output={ow}x{oh} vsync={}",
                r.name(),
                r.vsync()
            ));
            window_state("G: gpu renderer", &g);
            scene(&mut r, "gpu clear and fill", Some((0, 0)), &clear_and_fill);
            drop(r);
            t("G: gpu renderer destroyed");
            trace_events("G: gpu renderer destroyed", 300);
        }
        hints::reset(hints::GPU_DRIVER);
        g.destroy();
        t("G: destroyed");
        trace_events("G: destroyed", 500);
    }
}
