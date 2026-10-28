// Tests of Metal on the UIKit video driver: its Metal views, the Metal
// renderer on its windows, and the GPU API there.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver only starts in an app, so this is a program of
//! its own (`harness = false`), run as an app on the iOS Simulator by
//! `tools/ios_run.py` (the `__DATA,__sdl3_runner` section below), as the
//! other `uikit_*` programs are. Its main function traces:
//!
//! * window M (Metal): its Metal view (`metal_create_view()`, an
//!   `SDL_uikitmetalview` that becomes the view controller's view, with
//!   the window's `SDL_PROP_WINDOW_UIKIT_METAL_VIEW_TAG_NUMBER` as its tag)
//!   and the view's `CAMetalLayer` (`metal_get_layer()`: its scale,
//!   drawable size, pixel format), and the view destroyed (the window's
//!   own view comes back);
//! * window H (Metal, resizable, high pixel density): its view's drawable
//!   against the window's size in pixels, turned in-process to landscape
//!   right and back to portrait (`requestGeometryUpdateWithPreferences:`,
//!   iOS 16), with the `WINDOW_METAL_VIEW_RESIZED` events;
//! * the Metal renderer on H (the first renderer tried): its name, output
//!   size, maximum texture size, texture formats, vsync, device, command
//!   queue, layer and command encoder; the scenes the other renderers'
//!   tests draw (shapes, lines, viewports and clipping, blend modes,
//!   custom ones too, textures of each format it takes, packed 16-bit,
//!   10-bit and float ones, palettes, YUV and NV12, streaming and locked
//!   textures, render targets, geometry, linear scaling, debug text) and
//!   partial reads, each read back and hashed; vsync 1, 0, 2 and -1; the
//!   drawable following the window as it turns; and the renderer made
//!   again, which takes the window's Metal view;
//! * the GPU API: the simulator's Metal device is below what its Metal
//!   backend asks for, so making a device, and a "gpu" renderer, fails
//!   (as upstream's says, "Device does not meet the hardware requirements
//!   for SDL_GPU Metal").
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an iPhone 16 with iOS 18.5):
//! the hashes are of the same pixels. Each scene is also drawn by the
//! software renderer and compared with it (within the tolerances the other
//! renderers' tests allow), and the custom blend mode's pixels are
//! checked. Upstream gives every window that asks for no graphics API the
//! OpenGL flag on UIKit (OpenGL ES isn't translated yet), so the flags are
//! traced without it, and the mice's device events are left out.
//! Elsewhere this program does nothing.

#[cfg(not(target_os = "ios"))]
fn main() {}

/// The runner's words: run as an app.
#[cfg(target_os = "ios")]
#[used]
#[link_section = "__DATA,__sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(target_os = "ios")]
mod uikit_support;

#[cfg(target_os = "ios")]
fn main() {
    ios::main()
}

#[cfg(target_os = "ios")]
mod ios {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, WindowID};
    use sdl3::init::{self, InitFlags};
    use sdl3::render::{Renderer, TextureAccess, TextureAddressMode, Vertex};
    use sdl3::video::{
        self, BlendFactor, BlendMode, BlendOperation, Color, FColor, FPoint, FRect, PixelFormat,
        Rect, ScaleMode, Surface, Window,
    };

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::{check_trace, checked, elapsed, event_name, exit};

    /// Upstream's trace (the C program, on an iPhone 16 simulator with
    /// iOS 18.5).
    const EXPECTED: &str = "\
init video: ok
video driver: uikit
application state: 0
M: create 320x480 metal: ok
M: created: events: M:WINDOW_SAFE_AREA_CHANGED(0,0), M:WINDOW_MOUSE_ENTER(0,0), M:WINDOW_FOCUS_GAINED(0,0), M:WINDOW_SHOWN(0,0), M:WINDOW_PIXEL_SIZE_CHANGED(393,852)
M: created: size=393x852 pixels=393x852 density=1 flags=0x20000600 metal_view_tag=255
M: created: root view: SDL_uikitview tag=0 frame=0,0 393x852
M: layer of no view: Parameter 'view' is invalid
M: create view: ok
M: view: events: M:WINDOW_SAFE_AREA_CHANGED(0,0)
M: view: view: SDL_uikitmetalview super=SDL_uikitview tag=255 root=yes frame=0,0 393x852
M: view: layer: CAMetalLayer same=yes scale=1 drawable=393x852 pixels=393x852 format=80 framebuffer_only=yes device=yes
M: view: size=393x852 pixels=393x852 density=1 flags=0x20000600 metal_view_tag=255
M: view: root view: SDL_uikitmetalview tag=255 frame=0,0 393x852
M: view destroyed
M: view destroyed: events: M:WINDOW_SAFE_AREA_CHANGED(0,0)
M: view destroyed: size=393x852 pixels=393x852 density=1 flags=0x20000600 metal_view_tag=255
M: view destroyed: root view: SDL_uikitview tag=0 frame=0,0 393x852
M: destroyed
M: destroyed: events: M:WINDOW_DESTROYED(0,0)
H: create 320x480 metal resizable high-density: ok
H: created: events: H:WINDOW_SAFE_AREA_CHANGED(0,0), H:WINDOW_MOUSE_ENTER(0,0), H:WINDOW_FOCUS_GAINED(0,0), H:WINDOW_SHOWN(0,0), H:WINDOW_PIXEL_SIZE_CHANGED(1179,2556), H:WINDOW_DISPLAY_SCALE_CHANGED(0,0)
H: created: size=393x852 pixels=1179x2556 density=3 flags=0x20002620 metal_view_tag=255
H: created: root view: SDL_uikitview tag=0 frame=0,0 393x852
H: create view: ok
H: view: events: H:WINDOW_SAFE_AREA_CHANGED(0,0)
H: view: view: SDL_uikitmetalview super=SDL_uikitview tag=255 root=yes frame=0,0 393x852
H: view: layer: CAMetalLayer same=yes scale=3 drawable=1179x2556 pixels=1179x2556 format=80 framebuffer_only=yes device=yes
H: landscape right: events: DISPLAY_DESKTOP_MODE_CHANGED(1,852), DISPLAY_CURRENT_MODE_CHANGED(1,852), DISPLAY_ORIENTATION(1,1), H:WINDOW_METAL_VIEW_RESIZED(0,0), H:WINDOW_RESIZED(852,393), H:WINDOW_PIXEL_SIZE_CHANGED(2556,1179), H:WINDOW_SAFE_AREA_CHANGED(0,0)
H: landscape right: view: SDL_uikitmetalview super=SDL_uikitview tag=255 root=yes frame=0,0 852x393
H: landscape right: layer: CAMetalLayer same=yes scale=3 drawable=2556x1179 pixels=2556x1179 format=80 framebuffer_only=yes device=yes
H: portrait: events: DISPLAY_DESKTOP_MODE_CHANGED(1,393), DISPLAY_CURRENT_MODE_CHANGED(1,393), DISPLAY_ORIENTATION(1,3), H:WINDOW_METAL_VIEW_RESIZED(0,0), H:WINDOW_RESIZED(393,852), H:WINDOW_PIXEL_SIZE_CHANGED(1179,2556), H:WINDOW_SAFE_AREA_CHANGED(0,0)
H: portrait: view: SDL_uikitmetalview super=SDL_uikitview tag=255 root=yes frame=0,0 393x852
H: portrait: layer: CAMetalLayer same=yes scale=3 drawable=1179x2556 pixels=1179x2556 format=80 framebuffer_only=yes device=yes
H: view destroyed
H: view destroyed: events: H:WINDOW_SAFE_AREA_CHANGED(0,0)
H: view destroyed: size=393x852 pixels=1179x2556 density=3 flags=0x20002620 metal_view_tag=255
H: view destroyed: root view: SDL_uikitview tag=0 frame=0,0 393x852
H: create renderer: ok
H: renderer: events: H:WINDOW_SAFE_AREA_CHANGED(0,0)
H: renderer: name=metal output=1179x2556 pixels=1179x2556 max_texture_size=8192 vsync=0
H: renderer: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_ABGR2101010, SDL_PIXELFORMAT_RGBA64_FLOAT, SDL_PIXELFORMAT_RGBA128_FLOAT, SDL_PIXELFORMAT_RGB565, SDL_PIXELFORMAT_RGBA5551, SDL_PIXELFORMAT_ARGB1555, SDL_PIXELFORMAT_RGBA4444, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21, SDL_PIXELFORMAT_P010, SDL_PIXELFORMAT_I0FL, SDL_PIXELFORMAT_I4FL
H: renderer: device=Apple iOS simulator GPU queue=yes queue device=yes
H: renderer: layer: CAMetalLayer drawable=1179x2556 format=80 framebuffer_only=no device=yes window flags=0x20002620
H: renderer: root view: SDL_uikitmetalview tag=255 layer=yes
H: renderer: command encoder: yes
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
packed 16-bit formats: listed, not drawn
scene packed SDL_PIXELFORMAT_ABGR2101010: 64x48 e9c8703d
scene packed SDL_PIXELFORMAT_RGBA64_FLOAT: 64x48 bf4cacb5
scene packed SDL_PIXELFORMAT_RGBA128_FLOAT: 64x48 9a74c64d
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
scene debug text: 64x48 38bdb28c
partial read: 8x8 f24b7925
target read: 20x10 da6561f5
H: vsync 1: ok, vsync=1
H: vsync 0: ok, vsync=0
H: vsync 2: error: That operation is not supported, vsync=0
H: vsync -1: error: That operation is not supported, vsync=0
H: scenes: events: (none)
H: renderer landscape right: events: DISPLAY_DESKTOP_MODE_CHANGED(1,852), DISPLAY_CURRENT_MODE_CHANGED(1,852), DISPLAY_ORIENTATION(1,1), H:WINDOW_METAL_VIEW_RESIZED(0,0), H:WINDOW_RESIZED(852,393), H:WINDOW_PIXEL_SIZE_CHANGED(2556,1179), H:WINDOW_SAFE_AREA_CHANGED(0,0)
H: renderer landscape right: name=metal output=2556x1179 pixels=2556x1179 max_texture_size=8192 vsync=0
H: renderer landscape right: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_ABGR2101010, SDL_PIXELFORMAT_RGBA64_FLOAT, SDL_PIXELFORMAT_RGBA128_FLOAT, SDL_PIXELFORMAT_RGB565, SDL_PIXELFORMAT_RGBA5551, SDL_PIXELFORMAT_ARGB1555, SDL_PIXELFORMAT_RGBA4444, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21, SDL_PIXELFORMAT_P010, SDL_PIXELFORMAT_I0FL, SDL_PIXELFORMAT_I4FL
H: renderer landscape right: device=Apple iOS simulator GPU queue=yes queue device=yes
H: renderer landscape right: layer: CAMetalLayer drawable=2556x1179 format=80 framebuffer_only=no device=yes window flags=0x20002620
H: renderer landscape right: root view: SDL_uikitmetalview tag=255 layer=yes
H: renderer landscape right corner: 64x48 bf597dc5
H: renderer portrait: events: DISPLAY_DESKTOP_MODE_CHANGED(1,393), DISPLAY_CURRENT_MODE_CHANGED(1,393), DISPLAY_ORIENTATION(1,3), H:WINDOW_METAL_VIEW_RESIZED(0,0), H:WINDOW_RESIZED(393,852), H:WINDOW_PIXEL_SIZE_CHANGED(1179,2556), H:WINDOW_SAFE_AREA_CHANGED(0,0)
H: renderer portrait: name=metal output=1179x2556 pixels=1179x2556 max_texture_size=8192 vsync=0
H: renderer portrait: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_ABGR2101010, SDL_PIXELFORMAT_RGBA64_FLOAT, SDL_PIXELFORMAT_RGBA128_FLOAT, SDL_PIXELFORMAT_RGB565, SDL_PIXELFORMAT_RGBA5551, SDL_PIXELFORMAT_ARGB1555, SDL_PIXELFORMAT_RGBA4444, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21, SDL_PIXELFORMAT_P010, SDL_PIXELFORMAT_I0FL, SDL_PIXELFORMAT_I4FL
H: renderer portrait: device=Apple iOS simulator GPU queue=yes queue device=yes
H: renderer portrait: layer: CAMetalLayer drawable=1179x2556 format=80 framebuffer_only=no device=yes window flags=0x20002620
H: renderer portrait: root view: SDL_uikitmetalview tag=255 layer=yes
scene again clear and fill: 64x48 5fe1f24a
H: renderer destroyed
H: renderer destroyed: events: (none)
H: renderer destroyed: size=393x852 pixels=1179x2556 density=3 flags=0x20002620 metal_view_tag=255
H: renderer destroyed: root view: SDL_uikitmetalview tag=255 frame=0,0 393x852
H: create metal renderer again: ok
H: renderer again: same view=yes
H: renderer again: name=metal output=1179x2556 pixels=1179x2556 max_texture_size=8192 vsync=0
H: renderer again: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_ABGR2101010, SDL_PIXELFORMAT_RGBA64_FLOAT, SDL_PIXELFORMAT_RGBA128_FLOAT, SDL_PIXELFORMAT_RGB565, SDL_PIXELFORMAT_RGBA5551, SDL_PIXELFORMAT_ARGB1555, SDL_PIXELFORMAT_RGBA4444, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21, SDL_PIXELFORMAT_P010, SDL_PIXELFORMAT_I0FL, SDL_PIXELFORMAT_I4FL
H: renderer again: device=Apple iOS simulator GPU queue=yes queue device=yes
H: renderer again: layer: CAMetalLayer drawable=1179x2556 format=80 framebuffer_only=no device=yes window flags=0x20002620
H: renderer again: root view: SDL_uikitmetalview tag=255 layer=yes
scene renderer again clear and fill: 64x48 5fe1f24a
H: renderer again destroyed
H: renderer again destroyed: events: (none)
gpu: supports MSL on metal: yes
gpu: create metal device: error: Device does not meet the hardware requirements for SDL_GPU Metal
H: create gpu renderer: error: Device does not meet the hardware requirements for SDL_GPU Metal
H: gpu: events: (none)
H: destroyed
H: destroyed: events: H:WINDOW_DESTROYED(0,0)
quit
";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn t(line: impl Into<String>) {
        let line = line.into();
        println!("trace: {line}");
        TRACE.lock().unwrap().push(line);
    }

    /// What differed from the software renderer (checked after the trace).
    static MISMATCHES: Mutex<Vec<String>> = Mutex::new(Vec::new());

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

    /// The window's sizes and flags (without the OpenGL flag, see the
    /// module's description), its Metal view tag and its view
    /// controller's view.
    fn window_state(label: &str, w: &Window) {
        let (ww, wh) = w.size().unwrap_or_default();
        let (pw, ph) = w.size_in_pixels().unwrap_or_default();
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
        // SAFETY: a UIView property.
        let view_tag: isize = if v.is_null() {
            0
        } else {
            unsafe { objc::send0(v, "tag") }
        };
        t(format!(
            "{label}: root view: {} tag={view_tag} frame={}",
            objc::class_name(v),
            rect(frame_of(v))
        ));
    }

    /// A Metal view and its layer.
    fn view_state(label: &str, w: &Window, view: usize) {
        let v = view as Id;
        // SAFETY: a UIView property.
        let tag: isize = unsafe { objc::send0(v, "tag") };
        t(format!(
            "{label}: view: {} super={} tag={tag} root={} frame={}",
            objc::class_name(v),
            objc::superclass_name(v),
            yes_no(root_view_of(w) == v),
            rect(frame_of(v))
        ));
        let layer = video::vulkan::metal_get_layer(view).unwrap_or(0) as Id;
        let (pw, ph) = w.size_in_pixels().unwrap_or_default();
        // SAFETY: UIView's and CAMetalLayer's properties.
        unsafe {
            let view_layer: Id = objc::send0(v, "layer");
            let scale: f64 = objc::send0(layer, "contentsScale");
            let drawable: CGSize = objc::send0(layer, "drawableSize");
            let format: usize = objc::send0(layer, "pixelFormat");
            let framebuffer_only: bool = objc::send0(layer, "framebufferOnly");
            let device: Id = objc::send0(layer, "device");
            t(format!(
                "{label}: layer: {} same={} scale={scale} drawable={}x{} pixels={pw}x{ph} format={format} framebuffer_only={} device={}",
                objc::class_name(layer),
                yes_no(layer == view_layer),
                drawable.w,
                drawable.h,
                yes_no(framebuffer_only),
                yes_no(!device.is_null())
            ));
        }
    }

    /// Ask the window's scene to turn to an orientation (a
    /// `UIInterfaceOrientationMask`), and trace the events that come.
    fn turn(w: &Window, label: &str, mask: usize) {
        let mut seen = Vec::new();
        println!("uikit_metal: {:.3} s: turning to {label}", elapsed());
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
                std::ptr::null_mut::<std::ffi::c_void>(),
            );
        }
        pump_until(
            &mut seen,
            "DISPLAY_ORIENTATION",
            Duration::from_secs(5),
            1500,
        );
        println!("uikit_metal: {:.3} s: turned", elapsed());
        t(format!("{label}: events: {}", list(seen)));
    }

    // ------------------------------------------------------------------
    // The scenes (the C program draws the same ones)

    const RW: i32 = 64;
    const RH: i32 = 48;

    /// The pixels of a surface as tightly packed RGBA32.
    fn rgba(surface: &Surface<'_>) -> (i32, i32, Vec<u8>) {
        let s = surface.convert(PixelFormat::RGBA32).unwrap();
        let (w, h, pitch) = (s.width(), s.height(), s.pitch() as usize);
        let pixels = s.pixels().unwrap();
        let mut out = Vec::new();
        for y in 0..h as usize {
            out.extend_from_slice(&pixels[y * pitch..y * pitch + w as usize * 4]);
        }
        (w, h, out)
    }

    /// 32-bit FNV-1a.
    fn fnv(bytes: &[u8], mut h: u32) -> u32 {
        for &b in bytes {
            h ^= b as u32;
            h = h.wrapping_mul(16777619);
        }
        h
    }

    /// Trace the hash of the pixels (and print each row's, for comparing
    /// by hand).
    fn hash_pixels(label: &str, image: &(i32, i32, Vec<u8>)) {
        let (w, h, pixels) = image;
        let mut hash = 2166136261;
        let mut rows = String::new();
        for row in pixels.chunks(*w as usize * 4) {
            hash = fnv(row, hash);
            rows.push_str(&format!(" {:08x}", fnv(row, 2166136261)));
        }
        println!("rows {label}:{rows}");
        t(format!("{label}: {w}x{h} {hash:08x}"));
    }

    /// Trace a read back, or its error.
    fn hash_read(label: &str, read: sdl3::Result<Surface<'static>>) -> Option<(i32, i32, Vec<u8>)> {
        match read {
            Ok(s) => {
                let image = rgba(&s);
                hash_pixels(label, &image);
                Some(image)
            }
            Err(e) => {
                t(format!("{label}: read: error: {e}"));
                None
            }
        }
    }

    /// Compare two images: at most `max_bad` pixels may have a color
    /// channel off by more than `tolerance` (noted, not panicking, so the
    /// trace is compared first).
    fn compare(
        what: &str,
        a: &(i32, i32, Vec<u8>),
        b: &(i32, i32, Vec<u8>),
        tolerance: u8,
        max_bad: usize,
    ) {
        if (a.0, a.1) != (b.0, b.1) {
            MISMATCHES
                .lock()
                .unwrap()
                .push(format!("{what}: sizes {}x{} and {}x{}", a.0, a.1, b.0, b.1));
            return;
        }
        let mut bad = 0;
        let mut first = None;
        for (i, (pa, pb)) in a.2.chunks(4).zip(b.2.chunks(4)).enumerate() {
            if (0..3).any(|c| pa[c].abs_diff(pb[c]) > tolerance) {
                bad += 1;
                if first.is_none() {
                    first = Some((i as i32 % a.0, i as i32 / a.0, pa.to_vec(), pb.to_vec()));
                }
            }
        }
        if bad > max_bad {
            MISMATCHES.lock().unwrap().push(format!(
                "{what}: {bad} pixels differ (first at {first:?}: metal vs software)"
            ));
        }
    }

    type Scene<'a> = &'a dyn Fn(&mut Renderer) -> sdl3::Result<()>;

    /// Draw a scene, read back the top left RWxRH pixels (traced as their
    /// hash), present; and compare with the software renderer's pixels
    /// (`None`: not compared).
    fn scene(r: &mut Renderer, name: &str, compare_with: Option<(u8, usize)>, draw: Scene<'_>) {
        let label = format!("scene {name}");
        if let Err(e) = draw(r) {
            t(format!("{label}: error: {e}"));
        }
        let image = hash_read(&label, r.read_pixels(Some(&Rect::new(0, 0, RW, RH))));
        let _ = r.present();
        if let (Some(image), Some((tolerance, max_bad))) = (image, compare_with) {
            let mut sw =
                Renderer::software(Surface::new(RW, RH, PixelFormat::ARGB8888).unwrap()).unwrap();
            match draw(&mut sw) {
                Ok(()) => {
                    let b = rgba(&sw.read_pixels(None).unwrap());
                    compare(&label, &image, &b, tolerance, max_bad);
                }
                Err(e) => MISMATCHES
                    .lock()
                    .unwrap()
                    .push(format!("{label}: software: {e}")),
            }
        }
    }

    /// A 16x16 pattern with varying alpha.
    fn pattern_color(x: i32, y: i32) -> Color {
        Color::new(
            (x * 16) as u8,
            (y * 16) as u8,
            ((x + y) * 8) as u8,
            if (x + y) % 3 == 0 {
                255
            } else {
                (x * 8 + y * 7) as u8
            },
        )
    }

    /// A 16x16 surface of the pattern.
    fn pattern_surface(format: PixelFormat) -> Surface<'static> {
        let mut s = Surface::new(16, 16, format).unwrap();
        for y in 0..16 {
            for x in 0..16 {
                s.write_pixel(x, y, pattern_color(x, y)).unwrap();
            }
        }
        s
    }

    /// YUV planes of the pattern (`IYUV`, `YV12`, `NV12`, `NV21`), from an
    /// RGB image of `w`x`h` through SDL's conversion.
    fn yuv_pixels(format: PixelFormat, w: i32, h: i32) -> Vec<u8> {
        let mut rgb = Surface::new(w, h, PixelFormat::XRGB8888).unwrap();
        for y in 0..h {
            for x in 0..w {
                rgb.write_pixel(x, y, pattern_color(x % 16, y % 16))
                    .unwrap();
            }
        }
        let size = (w * h + 2 * ((w + 1) / 2) * ((h + 1) / 2)) as usize;
        let mut out = vec![0u8; size];
        video::convert_pixels(
            w,
            h,
            PixelFormat::XRGB8888,
            rgb.pixels().unwrap(),
            rgb.pitch(),
            format,
            &mut out,
            w,
        )
        .unwrap();
        out
    }

    fn clear_and_fill(r: &mut Renderer) -> sdl3::Result<()> {
        r.set_draw_color(10, 20, 30, 255);
        r.clear()?;
        r.set_draw_color(200, 100, 50, 255);
        r.render_fill_rect(Some(&FRect::new(8.0, 8.0, 32.0, 24.0)))?;
        r.set_draw_color(0, 255, 0, 255);
        r.render_fill_rects(&[
            FRect::new(-5.0, -5.0, 10.0, 10.0),
            FRect::new(50.0, 40.0, 30.0, 30.0),
        ])?;
        r.set_draw_color(255, 255, 255, 255);
        r.render_rect(Some(&FRect::new(2.0, 30.0, 12.0, 10.0)))
    }

    fn points_and_lines(r: &mut Renderer) -> sdl3::Result<()> {
        r.set_draw_color(0, 0, 0, 255);
        r.clear()?;
        r.set_draw_color(255, 128, 0, 255);
        let points: Vec<FPoint> = (0..40)
            .map(|i| FPoint {
                x: (i * 7 % 64) as f32,
                y: (i * 5 % 48) as f32,
            })
            .collect();
        r.render_points(&points)?;
        r.set_draw_color(255, 255, 0, 255);
        r.render_line(0.0, 0.0, 63.0, 0.0)?;
        r.render_line(5.0, 3.0, 5.0, 40.0)?;
        r.render_line(10.0, 10.0, 40.0, 30.0)?;
        r.render_lines(&[
            FPoint { x: 50.0, y: 5.0 },
            FPoint { x: 60.0, y: 5.0 },
            FPoint { x: 60.0, y: 15.0 },
        ])
    }

    fn viewport_and_clip(r: &mut Renderer) -> sdl3::Result<()> {
        r.set_draw_color(0, 0, 0, 255);
        r.clear()?;
        r.set_viewport(Some(&Rect::new(10, 6, 40, 30)))?;
        r.set_draw_color(255, 0, 0, 255);
        r.render_fill_rect(None)?;
        r.set_clip_rect(Some(&Rect::new(5, 5, 12, 8)))?;
        r.set_draw_color(0, 0, 255, 255);
        r.render_fill_rect(Some(&FRect::new(0.0, 0.0, 30.0, 30.0)))?;
        r.set_clip_rect(None)?;
        r.set_viewport(None)
    }

    fn blend_modes(r: &mut Renderer) -> sdl3::Result<()> {
        r.set_draw_blend_mode(BlendMode::NONE)?;
        r.set_draw_color(10, 20, 30, 255);
        r.clear()?;
        for (i, mode) in [
            BlendMode::BLEND,
            BlendMode::ADD,
            BlendMode::MOD,
            BlendMode::MUL,
        ]
        .into_iter()
        .enumerate()
        {
            r.set_draw_blend_mode(mode)?;
            r.set_draw_color(220, 90, 40, 128);
            r.render_fill_rect(Some(&FRect::new(i as f32 * 12.0, 12.0, 10.0, 30.0)))?;
        }
        Ok(())
    }

    fn texture_scene(r: &mut Renderer, format: PixelFormat) -> sdl3::Result<()> {
        r.set_draw_blend_mode(BlendMode::NONE)?;
        r.set_draw_color(40, 50, 60, 255);
        r.clear()?;
        let tex = r.create_texture_from_surface(&mut pattern_surface(format))?;
        r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
        r.render_texture(tex, None, Some(&FRect::new(4.0, 4.0, 16.0, 16.0)))?;
        r.set_texture_color_mod(tex, 255, 128, 64)?;
        r.set_texture_alpha_mod(tex, 160)?;
        r.render_texture(tex, None, Some(&FRect::new(24.0, 8.0, 32.0, 32.0)))?;
        r.destroy_texture(tex);
        Ok(())
    }

    fn streaming_texture(r: &mut Renderer) -> sdl3::Result<()> {
        r.set_draw_color(0, 0, 0, 255);
        r.clear()?;
        let tex = r.create_texture(PixelFormat::ARGB8888, TextureAccess::Streaming, 16, 8)?;
        r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
        let mut pixels = vec![0u8; 16 * 8 * 4];
        for (i, p) in pixels.chunks_mut(4).enumerate() {
            p.copy_from_slice(&[(i * 3) as u8, (i * 5) as u8, 200, 255]);
        }
        r.update_texture(tex, None, &pixels, 64)?;
        r.render_texture(tex, None, Some(&FRect::new(0.0, 0.0, 16.0, 8.0)))?;
        for p in pixels.chunks_mut(4) {
            p[..3].iter_mut().for_each(|b| *b = b.wrapping_add(40));
        }
        r.update_texture(tex, None, &pixels, 64)?;
        r.render_texture(tex, None, Some(&FRect::new(20.0, 20.0, 16.0, 8.0)))?;
        r.destroy_texture(tex);
        Ok(())
    }

    fn render_target(r: &mut Renderer) -> sdl3::Result<()> {
        let target = r.create_texture(PixelFormat::ARGB8888, TextureAccess::Target, 32, 24)?;
        r.set_render_target(Some(target))?;
        r.set_draw_color(255, 0, 255, 255);
        r.clear()?;
        r.set_draw_color(0, 255, 0, 255);
        r.render_fill_rect(Some(&FRect::new(4.0, 4.0, 8.0, 8.0)))?;
        r.set_render_target(None)?;
        r.set_draw_color(0, 0, 0, 255);
        r.clear()?;
        r.set_texture_scale_mode(target, ScaleMode::Nearest)?;
        r.render_texture(target, None, Some(&FRect::new(8.0, 8.0, 32.0, 24.0)))?;
        r.destroy_texture(target);
        Ok(())
    }

    fn packed_texture(r: &mut Renderer, format: PixelFormat) -> sdl3::Result<()> {
        r.set_draw_blend_mode(BlendMode::NONE)?;
        r.set_draw_color(40, 50, 60, 255);
        r.clear()?;
        let mut s = pattern_surface(PixelFormat::ARGB8888).convert(format)?;
        let tex = r.create_texture_from_surface(&mut s)?;
        r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
        r.render_texture(tex, None, Some(&FRect::new(4.0, 4.0, 32.0, 32.0)))?;
        r.destroy_texture(tex);
        Ok(())
    }

    fn palette_texture(r: &mut Renderer) -> sdl3::Result<()> {
        r.set_draw_color(0, 0, 0, 255);
        r.clear()?;
        let mut s = Surface::new(16, 16, PixelFormat::INDEX8)?;
        let palette = s.create_palette()?;
        {
            let mut p = palette.write().unwrap();
            let colors: Vec<Color> = (0..256)
                .map(|i| Color::new(i as u8, (255 - i) as u8, (i * 7) as u8, 255))
                .collect();
            p.set_colors(0, &colors)?;
        }
        {
            let pitch = s.pitch() as usize;
            let px = s.pixels_mut().unwrap();
            for y in 0..16 {
                for x in 0..16 {
                    px[y * pitch + x] = (x * 16 + y) as u8;
                }
            }
        }
        let tex = r.create_texture_from_surface(&mut s)?;
        r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
        r.render_texture(tex, None, Some(&FRect::new(0.0, 0.0, 16.0, 16.0)))?;
        r.render_texture(tex, None, Some(&FRect::new(20.0, 4.0, 32.0, 32.0)))?;
        r.destroy_texture(tex);
        Ok(())
    }

    fn yuv_texture(r: &mut Renderer, format: PixelFormat) -> sdl3::Result<()> {
        r.set_draw_color(0, 0, 0, 255);
        r.clear()?;
        let tex = r.create_texture(format, TextureAccess::Static, 32, 32)?;
        r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
        let pixels = yuv_pixels(format, 32, 32);
        r.update_texture(tex, None, &pixels, 32)?;
        r.render_texture(tex, None, Some(&FRect::new(8.0, 8.0, 32.0, 32.0)))?;
        r.destroy_texture(tex);
        Ok(())
    }

    fn iyuv_planes(r: &mut Renderer) -> sdl3::Result<()> {
        r.set_draw_color(0, 0, 0, 255);
        r.clear()?;
        let tex = r.create_texture(PixelFormat::IYUV, TextureAccess::Static, 32, 32)?;
        r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
        let pixels = yuv_pixels(PixelFormat::IYUV, 32, 32);
        let (y, rest) = pixels.split_at(32 * 32);
        let (u, v) = rest.split_at(16 * 16);
        r.update_yuv_texture(tex, None, y, 32, u, 16, v, 16)?;
        r.render_texture(tex, None, Some(&FRect::new(8.0, 8.0, 32.0, 32.0)))?;
        r.destroy_texture(tex);
        Ok(())
    }

    fn nv12_planes(r: &mut Renderer) -> sdl3::Result<()> {
        r.set_draw_color(0, 0, 0, 255);
        r.clear()?;
        let tex = r.create_texture(PixelFormat::NV12, TextureAccess::Static, 32, 32)?;
        r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
        let pixels = yuv_pixels(PixelFormat::NV12, 32, 32);
        let (y, uv) = pixels.split_at(32 * 32);
        r.update_nv_texture(tex, None, y, 32, uv, 32)?;
        r.render_texture(tex, None, Some(&FRect::new(8.0, 8.0, 32.0, 32.0)))?;
        r.destroy_texture(tex);
        Ok(())
    }

    fn locked_texture(r: &mut Renderer) -> sdl3::Result<()> {
        r.set_draw_color(0, 0, 0, 255);
        r.clear()?;
        let tex = r.create_texture(PixelFormat::ABGR8888, TextureAccess::Streaming, 16, 16)?;
        r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
        {
            let mut lock = r.lock_texture(tex, Some(&Rect::new(4, 2, 8, 10)))?;
            let pitch = lock.pitch() as usize;
            let px = lock.pixels();
            for y in 0..10 {
                for x in 0..8 {
                    px[y * pitch + x * 4..][..4].copy_from_slice(&[
                        (x * 30) as u8,
                        (y * 25) as u8,
                        99,
                        255,
                    ]);
                }
            }
        }
        r.render_texture(tex, None, Some(&FRect::new(0.0, 0.0, 32.0, 32.0)))?;
        r.destroy_texture(tex);
        Ok(())
    }

    fn locked_nv12(r: &mut Renderer) -> sdl3::Result<()> {
        r.set_draw_color(0, 0, 0, 255);
        r.clear()?;
        let tex = r.create_texture(PixelFormat::NV12, TextureAccess::Streaming, 32, 32)?;
        r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
        let pixels = yuv_pixels(PixelFormat::NV12, 32, 32);
        {
            let mut lock = r.lock_texture(tex, None)?;
            lock.pixels()[..pixels.len()].copy_from_slice(&pixels);
        }
        r.render_texture(tex, None, Some(&FRect::new(8.0, 8.0, 32.0, 32.0)))?;
        r.destroy_texture(tex);
        Ok(())
    }

    fn custom_blend(r: &mut Renderer) -> sdl3::Result<()> {
        let mode = BlendMode::compose_custom(
            BlendFactor::One,
            BlendFactor::One,
            BlendOperation::RevSubtract,
            BlendFactor::Zero,
            BlendFactor::One,
            BlendOperation::Add,
        );
        r.set_draw_blend_mode(BlendMode::NONE)?;
        r.set_draw_color(100, 150, 200, 255);
        r.clear()?;
        r.set_draw_blend_mode(mode)?;
        r.set_draw_color(60, 60, 60, 128);
        r.render_fill_rect(Some(&FRect::new(10.0, 10.0, 30.0, 20.0)))?;
        r.set_draw_blend_mode(BlendMode::NONE)
    }

    fn vertex(x: f32, y: f32, color: FColor, u: f32, v: f32) -> Vertex {
        Vertex {
            position: FPoint { x, y },
            color,
            tex_coord: FPoint { x: u, y: v },
        }
    }

    fn wrapping_geometry(r: &mut Renderer) -> sdl3::Result<()> {
        r.set_draw_color(0, 0, 0, 255);
        r.clear()?;
        let tex = r.create_texture_from_surface(&mut pattern_surface(PixelFormat::ARGB8888))?;
        r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
        r.set_texture_blend_mode(tex, BlendMode::NONE)?;
        r.set_texture_address_mode(TextureAddressMode::Wrap, TextureAddressMode::Wrap);
        let white = FColor::new(1.0, 1.0, 1.0, 1.0);
        let verts = [
            vertex(0.0, 0.0, white, 0.0, 0.0),
            vertex(32.0, 0.0, white, 2.0, 0.0),
            vertex(32.0, 32.0, white, 2.0, 2.0),
            vertex(0.0, 32.0, white, 0.0, 2.0),
        ];
        let result = r.render_geometry(Some(tex), &verts, Some(&[0, 1, 2, 0, 2, 3]));
        r.set_texture_address_mode(TextureAddressMode::Auto, TextureAddressMode::Auto);
        r.destroy_texture(tex);
        result
    }

    fn colored_geometry(r: &mut Renderer) -> sdl3::Result<()> {
        r.set_draw_color(0, 0, 0, 255);
        r.clear()?;
        let red = FColor::new(1.0, 0.0, 0.0, 1.0);
        let green = FColor::new(0.0, 1.0, 0.0, 1.0);
        let blue = FColor::new(0.0, 0.0, 1.0, 1.0);
        let verts = [
            vertex(4.0, 4.0, red, 0.0, 0.0),
            vertex(60.0, 10.0, green, 0.0, 0.0),
            vertex(20.0, 44.0, blue, 0.0, 0.0),
        ];
        r.render_geometry(None, &verts, None)
    }

    fn linear_scaling(r: &mut Renderer) -> sdl3::Result<()> {
        r.set_draw_color(0, 0, 0, 255);
        r.clear()?;
        let tex = r.create_texture_from_surface(&mut pattern_surface(PixelFormat::XRGB8888))?;
        r.set_texture_scale_mode(tex, ScaleMode::Linear)?;
        r.render_texture(tex, None, Some(&FRect::new(0.0, 0.0, 48.0, 40.0)))?;
        r.destroy_texture(tex);
        Ok(())
    }

    fn debug_text(r: &mut Renderer) -> sdl3::Result<()> {
        r.set_draw_color(0, 0, 0, 255);
        r.clear()?;
        r.set_draw_color(255, 255, 255, 255);
        r.render_debug_text(2.0, 2.0, "Metal")?;
        r.render_debug_text(2.0, 20.0, "SDL 3")
    }

    /// The renderer's scenes.
    fn scenes(r: &mut Renderer) {
        let formats = r.texture_formats().to_vec();
        scene(r, "clear and fill", Some((0, 0)), &clear_and_fill);
        scene(r, "points and lines", Some((0, 4)), &points_and_lines);
        scene(r, "viewport and clip", Some((0, 0)), &viewport_and_clip);
        scene(r, "blend modes", Some((2, 0)), &blend_modes);
        for format in [
            PixelFormat::ARGB8888,
            PixelFormat::ABGR8888,
            PixelFormat::RGB24,
            PixelFormat::XRGB8888,
        ] {
            scene(
                r,
                &format!("texture {}", format.name()),
                Some((2, 0)),
                &|r| texture_scene(r, format),
            );
        }
        scene(r, "streaming texture", Some((0, 0)), &streaming_texture);
        scene(r, "render target", Some((0, 0)), &render_target);
        let mut packed = vec![
            PixelFormat::ABGR2101010,
            PixelFormat::RGBA64_FLOAT,
            PixelFormat::RGBA128_FLOAT,
        ];
        // (the simulator's Metal refuses the packed 16-bit formats the
        // renderer lists: a validation assertion ends the process, as with
        // upstream's renderer)
        if formats.contains(&PixelFormat::RGB565) && cfg!(target_abi = "sim") {
            t("packed 16-bit formats: listed, not drawn");
        } else if formats.contains(&PixelFormat::RGB565) {
            packed.extend([
                PixelFormat::RGB565,
                PixelFormat::RGBA5551,
                PixelFormat::ARGB1555,
                PixelFormat::RGBA4444,
            ]);
        }
        for format in packed {
            // (the surface's quantization is the texture's on both sides)
            scene(
                r,
                &format!("packed {}", format.name()),
                Some((2, 0)),
                &|r| packed_texture(r, format),
            );
        }
        scene(r, "palette texture", Some((0, 0)), &palette_texture);
        // (the shaders' conversion against SDL's: a few levels apart at the
        // color edges, where the chroma is subsampled)
        for format in [
            PixelFormat::IYUV,
            PixelFormat::YV12,
            PixelFormat::NV12,
            PixelFormat::NV21,
        ] {
            scene(r, &format!("yuv {}", format.name()), Some((6, 24)), &|r| {
                yuv_texture(r, format)
            });
        }
        scene(r, "IYUV planes", Some((6, 24)), &iyuv_planes);
        scene(r, "NV12 planes", Some((6, 24)), &nv12_planes);
        scene(r, "locked texture", Some((0, 0)), &locked_texture);
        scene(r, "locked NV12 texture", Some((6, 24)), &locked_nv12);
        // (the software renderer has no custom blend modes: the pixels
        // are checked below)
        scene(r, "custom blend mode", None, &custom_blend);
        scene(r, "wrapping geometry", Some((2, 0)), &wrapping_geometry);
        // (the interpolated colors, a level or two apart, and the edges)
        scene(r, "colored geometry", Some((3, 120)), &colored_geometry);
        scene(r, "linear scaling", Some((6, 40)), &linear_scaling);
        scene(r, "debug text", Some((0, 0)), &debug_text);

        // A partial read, and a read from a render target
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.set_draw_color(255, 0, 0, 255);
        r.render_fill_rect(Some(&FRect::new(10.0, 10.0, 4.0, 4.0)))
            .unwrap();
        hash_read("partial read", r.read_pixels(Some(&Rect::new(8, 8, 8, 8))));
        let _ = r.present();
        let target = r
            .create_texture(PixelFormat::ABGR8888, TextureAccess::Target, 20, 10)
            .unwrap();
        r.set_render_target(Some(target)).unwrap();
        r.set_draw_color(0, 0, 255, 255);
        r.clear().unwrap();
        hash_read("target read", r.read_pixels(None));
        r.set_render_target(None).unwrap();
        r.destroy_texture(target);
    }

    /// The custom blend mode's pixels: dst - src for the color.
    fn check_custom_blend(r: &mut Renderer) {
        custom_blend(r).unwrap();
        let s = r.read_pixels(None).unwrap();
        assert_eq!(s.read_pixel(20, 20).unwrap(), Color::new(40, 90, 140, 255));
        assert_eq!(s.read_pixel(5, 5).unwrap(), Color::new(100, 150, 200, 255));
        r.present().unwrap();
    }

    /// The renderer's name, sizes, formats and properties.
    fn renderer_state(label: &str, r: &mut Renderer, w: &Window) {
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
        let device = props
            .get_number(sdl3::render::PROP_RENDERER_METAL_DEVICE_POINTER)
            .unwrap_or(0) as usize as Id;
        let queue = props
            .get_number(sdl3::render::PROP_RENDERER_METAL_COMMAND_QUEUE_POINTER)
            .unwrap_or(0) as usize as Id;
        let layer = r
            .metal_layer()
            .map_or(std::ptr::null_mut(), |l| l.as_ptr() as Id);
        // SAFETY: MTLDevice's, MTLCommandQueue's and CAMetalLayer's
        // properties.
        unsafe {
            let name = if device.is_null() {
                None
            } else {
                objc::string(objc::send0(device, "name"))
            };
            let queue_device: Id = if queue.is_null() {
                std::ptr::null_mut()
            } else {
                objc::send0(queue, "device")
            };
            t(format!(
                "{label}: device={} queue={} queue device={}",
                name.as_deref().unwrap_or("nil"),
                yes_no(!queue.is_null()),
                yes_no(queue_device == device)
            ));
            let drawable: CGSize = objc::send0(layer, "drawableSize");
            let format: usize = objc::send0(layer, "pixelFormat");
            let framebuffer_only: bool = objc::send0(layer, "framebufferOnly");
            let layer_device: Id = objc::send0(layer, "device");
            let flags = w.flags().unwrap_or_default().0 & !WindowFlags::OPENGL.0;
            t(format!(
                "{label}: layer: {} drawable={}x{} format={format} framebuffer_only={} device={} window flags={flags:#x}",
                objc::class_name(layer),
                drawable.w,
                drawable.h,
                yes_no(framebuffer_only),
                yes_no(layer_device == device)
            ));
            let v = root_view_of(w);
            let tag: isize = objc::send0(v, "tag");
            let view_layer: Id = objc::send0(v, "layer");
            t(format!(
                "{label}: root view: {} tag={tag} layer={}",
                objc::class_name(v),
                yes_no(view_layer == layer)
            ));
        }
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
        println!("uikit_metal: run_app returned {status}, which it never does");
        exit(1);
    }

    fn main_function() {
        const LANDSCAPE_RIGHT: usize = 1 << 3;
        const PORTRAIT: usize = 1 << 1;

        println!("uikit_metal: {:.3} s: main", elapsed());
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

        // M: a Metal window, its Metal view made and destroyed
        let r = Window::create("M", 320, 480, WindowFlags::METAL);
        t(format!("M: create 320x480 metal: {}", ok(&r)));
        let m = r.unwrap();
        name_window(&m, "M");
        trace_events("M: created", 500);
        window_state("M: created", &m);
        t(format!(
            "M: layer of no view: {}",
            match video::vulkan::metal_get_layer(0) {
                Ok(_) => "yes".to_string(),
                Err(e) => e.to_string(),
            }
        ));
        let r = video::vulkan::metal_create_view(&m);
        t(format!("M: create view: {}", ok(&r)));
        let mv = r.unwrap();
        trace_events("M: view", 500);
        view_state("M: view", &m, mv);
        window_state("M: view", &m);
        video::vulkan::metal_destroy_view(mv);
        t("M: view destroyed");
        trace_events("M: view destroyed", 300);
        window_state("M: view destroyed", &m);
        m.destroy();
        t("M: destroyed");
        trace_events("M: destroyed", 500);

        // H: a resizable high-density Metal window, turned with its view
        let r = Window::create(
            "H",
            320,
            480,
            WindowFlags::METAL | WindowFlags::RESIZABLE | WindowFlags::HIGH_PIXEL_DENSITY,
        );
        t(format!(
            "H: create 320x480 metal resizable high-density: {}",
            ok(&r)
        ));
        let h = r.unwrap();
        name_window(&h, "H");
        trace_events("H: created", 500);
        window_state("H: created", &h);
        let r = video::vulkan::metal_create_view(&h);
        t(format!("H: create view: {}", ok(&r)));
        let hv = r.unwrap();
        trace_events("H: view", 500);
        view_state("H: view", &h, hv);
        turn(&h, "H: landscape right", LANDSCAPE_RIGHT);
        view_state("H: landscape right", &h, hv);
        turn(&h, "H: portrait", PORTRAIT);
        view_state("H: portrait", &h, hv);
        video::vulkan::metal_destroy_view(hv);
        t("H: view destroyed");
        trace_events("H: view destroyed", 300);
        window_state("H: view destroyed", &h);

        // The Metal renderer on H (the first one tried)
        let r = Renderer::for_window(&h, None);
        t(format!("H: create renderer: {}", ok(&r)));
        let mut r = r.unwrap();
        trace_events("H: renderer", 500);
        renderer_state("H: renderer", &mut r, &h);
        t(format!(
            "H: renderer: command encoder: {}",
            yes_no(r.metal_command_encoder().is_some())
        ));
        let _ = r.present();
        scenes(&mut r);
        for want in [1, 0, 2, -1] {
            let result = r.set_vsync(want);
            t(format!(
                "H: vsync {want}: {}, vsync={}",
                ok(&result),
                r.vsync()
            ));
            r.set_draw_color(0, 0, 0, 255);
            let _ = r.clear();
            let _ = r.present();
        }
        trace_events("H: scenes", 300);

        // The drawable follows the window as it turns
        turn(&h, "H: renderer landscape right", LANDSCAPE_RIGHT);
        renderer_state("H: renderer landscape right", &mut r, &h);
        r.set_draw_color(0, 128, 0, 255);
        let _ = r.clear();
        let (ow, oh) = r.output_size().unwrap_or_default();
        hash_read(
            "H: renderer landscape right corner",
            r.read_pixels(Some(&Rect::new(ow - RW, oh - RH, RW, RH))),
        );
        let _ = r.present();
        turn(&h, "H: renderer portrait", PORTRAIT);
        renderer_state("H: renderer portrait", &mut r, &h);
        scene(
            &mut r,
            "again clear and fill",
            Some((0, 0)),
            &clear_and_fill,
        );
        check_custom_blend(&mut r);

        let view_before = root_view_of(&h);
        drop(r);
        t("H: renderer destroyed");
        trace_events("H: renderer destroyed", 300);
        window_state("H: renderer destroyed", &h);
        // Made again: the window's Metal view (GetWindowView())
        let r = Renderer::for_window(&h, Some("metal"));
        t(format!("H: create metal renderer again: {}", ok(&r)));
        let mut r = r.unwrap();
        t(format!(
            "H: renderer again: same view={}",
            yes_no(root_view_of(&h) == view_before)
        ));
        renderer_state("H: renderer again", &mut r, &h);
        scene(
            &mut r,
            "renderer again clear and fill",
            Some((0, 0)),
            &clear_and_fill,
        );
        drop(r);
        t("H: renderer again destroyed");
        trace_events("H: renderer again destroyed", 300);

        // The GPU API: the simulator's device isn't enough for its Metal
        // backend
        use sdl3::gpu::{Device, ShaderFormat};
        t(format!(
            "gpu: supports MSL on metal: {}",
            yes_no(sdl3::gpu::supports_shader_formats(
                ShaderFormat::MSL,
                Some("metal")
            ))
        ));
        let device = Device::new(
            ShaderFormat::MSL | ShaderFormat::METALLIB,
            false,
            Some("metal"),
        );
        t(format!("gpu: create metal device: {}", ok(&device)));
        drop(device);
        let gr = Renderer::for_window(&h, Some("gpu"));
        t(format!("H: create gpu renderer: {}", ok(&gr)));
        drop(gr);
        trace_events("H: gpu", 300);

        h.destroy();
        t("H: destroyed");
        trace_events("H: destroyed", 500);

        init::quit();
        t("quit");

        let mismatches = MISMATCHES.lock().unwrap();
        for m in mismatches.iter() {
            println!("uikit_metal: against the software renderer: {m}");
        }
        check_trace(&TRACE.lock().unwrap(), EXPECTED);
        assert!(
            mismatches.is_empty(),
            "the Metal renderer against the software renderer:\n{}",
            mismatches.join("\n")
        );
        println!("uikit_metal: all checks passed");
    }
}
