// Tests of OpenGL ES on the android video driver: its contexts on the
// activity's surface, and the OpenGL ES 2 renderer on its windows.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, whose main
//! function traces, against upstream's C SDL:
//!
//! * the OpenGL attributes' defaults (OpenGL ES 2.0), and an OpenGL
//!   window: its EGL surface (`SDL_PROP_WINDOW_ANDROID_SURFACE_POINTER`;
//!   `SDL_EGL_GetWindowSurface()` gives none, the driver having no
//!   `GL_GetEGLSurface`), a context on it, the GL strings
//!   (SwiftShader's, through the emulator), the attributes as SDL reads
//!   them back, the swap interval (EGL's, which SDL sets to 0 first), a
//!   clear read back and swapped;
//! * the OpenGL ES 2 renderer on the window: its name, output size,
//!   maximum texture size, texture formats and vsync; the scenes the other
//!   renderers' tests draw (`uikit_support::scenes`, each read back and
//!   hashed, and compared with the software renderer, within the
//!   tolerances the other renderers' tests allow), partial reads, the
//!   custom blend mode's pixels, and vsync 1 and 0;
//! * the context across a trip to the background: the activity's surface
//!   is destroyed once SDL's thread has kept the context and released it
//!   (the pause's backup), then made again; the context is made current
//!   with the new EGL surface when the app resumes (the restore, which
//!   would make a new context, and send `SDL_EVENT_RENDER_DEVICE_RESET`,
//!   if the context were lost): the application's and render events, and
//!   a scene drawn again.
//!
//! Upstream's renderer frees a program cache entry twice when it's
//! destroyed after these scenes (its cache list loses its tail: the
//! `Note (upstream)` on the Rust renderer's cache, a `Vec`), so the C
//! program is built with that list's two missing updates added, as on
//! iOS. The expected trace is upstream's C SDL's, from the same program in
//! C on the same emulator (API 35, Google APIs, x86-64). Elsewhere this
//! program does nothing.

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
    use sdl3::init::{self, InitFlags};
    use sdl3::render::Renderer;
    use sdl3::video::gl::{self, GlAttr, GlContext};
    use sdl3::video::{self, Window};

    use super::android_support::check_trace;
    use super::android_support::events::{self as ev, is_app_event};
    use super::scenes::{check_custom_blend, clear_and_fill, scene, scenes, t, MISMATCHES, TRACE};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
init: ok
video driver: android
defaults: profile=4 version=2.0 red=error: No GL driver has been loaded green=error: No GL driver has been loaded blue=error: No GL driver has been loaded alpha=error: No GL driver has been loaded depth=error: No GL driver has been loaded stencil=error: No GL driver has been loaded doublebuffer=1
window: ok
android surface pointer: yes, EGL window surface: no, the same: no
context: ok
current: yes
GL_VERSION: OpenGL ES 3.0 (OpenGL ES 3.0 SwiftShader 4.0.0.1)
GL_RENDERER: Android Emulator OpenGL ES Translator (Google SwiftShader)
GL_VENDOR: Google (Google Inc.)
GL_SHADING_LANGUAGE_VERSION: OpenGL ES GLSL ES 3.00
attributes: red=8 green=8 blue=8 alpha=8 depth=24 stencil=8 doublebuffer=1 version=2.0 profile=4
swap interval: 0
set swap interval 1: ok
swap interval: 1
set swap interval -1: error: Late swap tearing currently unsupported
set swap interval 0: ok
clear: 255,0,255,255
swap: ok
context destroyed
renderer: ok
renderer: name=opengles2 output=2400x1080 pixels=2400x1080 max_texture_size=8192 vsync=0
renderer: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_XRGB8888, SDL_PIXELFORMAT_XBGR8888, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21, SDL_PIXELFORMAT_EXTERNAL_OES
scene clear and fill: 64x48 5fe1f24a
scene points and lines: 64x48 75192d12
scene viewport and clip: 64x48 b09f79e5
scene blend modes: 64x48 ebb10095
scene texture SDL_PIXELFORMAT_ARGB8888: 64x48 34d06d0b
scene texture SDL_PIXELFORMAT_ABGR8888: 64x48 34d06d0b
scene texture SDL_PIXELFORMAT_RGB24: 64x48 4fb014c5
scene texture SDL_PIXELFORMAT_XRGB8888: 64x48 4fb014c5
scene streaming texture: 64x48 c2319ec5
scene render target: 64x48 9e5285c5
scene packed SDL_PIXELFORMAT_ABGR2101010: 64x48 8bdc6af5
scene packed SDL_PIXELFORMAT_RGBA64_FLOAT: 64x48 9a74c64d
scene packed SDL_PIXELFORMAT_RGBA128_FLOAT: 64x48 9a74c64d
scene palette texture: 64x48 65a75445
scene yuv SDL_PIXELFORMAT_IYUV: 64x48 1195fbbd
scene yuv SDL_PIXELFORMAT_YV12: 64x48 1195fbbd
scene yuv SDL_PIXELFORMAT_NV12: 64x48 1195fbbd
scene yuv SDL_PIXELFORMAT_NV21: 64x48 1195fbbd
scene IYUV planes: 64x48 1195fbbd
scene NV12 planes: 64x48 1195fbbd
scene locked texture: 64x48 76d14245
scene locked NV12 texture: 64x48 1195fbbd
scene custom blend mode: 64x48 af879425
scene wrapping geometry: 64x48 dd468f45
scene colored geometry: 64x48 fc2fd151
scene linear scaling: 64x48 5599895b
scene debug text: 64x48 1ecf4704
partial read: 8x8 f24b7925
target read: 20x10 da6561f5
set vsync 1: ok, vsync=1
set vsync 0: ok, vsync=0
trip: app events: WILL_ENTER_BACKGROUND(0,0), DID_ENTER_BACKGROUND(0,0), WILL_ENTER_FOREGROUND(0,0), DID_ENTER_FOREGROUND(0,0)
trip: render events: (none)
after the trip: name=opengles2 output=2400x1080 pixels=2400x1080 max_texture_size=8192 vsync=0
after the trip: formats: SDL_PIXELFORMAT_ARGB8888, SDL_PIXELFORMAT_ABGR8888, SDL_PIXELFORMAT_XRGB8888, SDL_PIXELFORMAT_XBGR8888, SDL_PIXELFORMAT_INDEX8, SDL_PIXELFORMAT_YV12, SDL_PIXELFORMAT_IYUV, SDL_PIXELFORMAT_I444, SDL_PIXELFORMAT_NV12, SDL_PIXELFORMAT_NV21, SDL_PIXELFORMAT_EXTERNAL_OES
scene after the trip: 64x48 5fe1f24a
renderer destroyed
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

    /// `glGetString(name)`.
    fn gl_string(name: u32) -> String {
        type GetString = unsafe extern "C" fn(u32) -> *const c_char;
        let Some(f) = gl::gl_get_proc_address("glGetString") else {
            return "(no glGetString)".to_string();
        };
        // SAFETY: glGetString's type; a context is current.
        unsafe {
            let f: GetString = std::mem::transmute(f);
            let s = f(name);
            if s.is_null() {
                "(null)".to_string()
            } else {
                CStr::from_ptr(s).to_string_lossy().into_owned()
            }
        }
    }

    /// Clear the current context's framebuffer to a color and read a pixel
    /// back.
    fn clear_and_read(r: f32, g: f32, b: f32) -> [u8; 4] {
        type ClearColor = unsafe extern "C" fn(f32, f32, f32, f32);
        type Clear = unsafe extern "C" fn(u32);
        type Finish = unsafe extern "C" fn();
        type ReadPixels = unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void);
        let get = |name: &str| gl::gl_get_proc_address(name).unwrap_or_else(|| panic!("{name}"));
        let mut pixel = [0u8; 4];
        // SAFETY: the GL functions have these types; a context is current;
        // the buffer holds one RGBA pixel.
        unsafe {
            let clear_color: ClearColor = std::mem::transmute(get("glClearColor"));
            let clear: Clear = std::mem::transmute(get("glClear"));
            let finish: Finish = std::mem::transmute(get("glFinish"));
            let read_pixels: ReadPixels = std::mem::transmute(get("glReadPixels"));
            clear_color(r, g, b, 1.0);
            clear(0x4000); // GL_COLOR_BUFFER_BIT
            finish();
            // GL_RGBA, GL_UNSIGNED_BYTE
            read_pixels(1, 1, 1, 1, 0x1908, 0x1401, pixel.as_mut_ptr().cast());
        }
        pixel
    }

    fn attr(a: GlAttr) -> String {
        gl::gl_get_attribute(a).map_or_else(|e| format!("error: {e}"), |v| v.to_string())
    }

    /// The renderer's name, sizes, formats and vsync.
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
        let formats: Vec<&str> = r.texture_formats().iter().map(|f| f.name()).collect();
        t(format!("{label}: formats: {}", formats.join(", ")));
    }

    pub(super) fn main() {
        t(format!("init: {}", result(&init::init(InitFlags::VIDEO))));
        ev::watch();
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap_or("(none)")
        ));
        t(format!(
            "defaults: profile={} version={}.{} red={} green={} blue={} alpha={} depth={} stencil={} doublebuffer={}",
            attr(GlAttr::ContextProfileMask),
            attr(GlAttr::ContextMajorVersion),
            attr(GlAttr::ContextMinorVersion),
            attr(GlAttr::RedSize),
            attr(GlAttr::GreenSize),
            attr(GlAttr::BlueSize),
            attr(GlAttr::AlphaSize),
            attr(GlAttr::DepthSize),
            attr(GlAttr::StencilSize),
            attr(GlAttr::DoubleBuffer)
        ));

        // An OpenGL window and a context on it
        let created = Window::create("android gles", 320, 240, WindowFlags::OPENGL);
        t(format!("window: {}", result(&created)));
        let w = created.expect("the window");
        ev::pump_for(300);
        ev::take();
        let surface = w
            .properties()
            .ok()
            .and_then(|p| p.get_number(video::window::PROP_WINDOW_ANDROID_SURFACE_POINTER))
            .unwrap_or(0);
        let egl_surface = gl::egl_window_surface(&w).ok().flatten();
        t(format!(
            "android surface pointer: {}, EGL window surface: {}, the same: {}",
            yes(surface != 0),
            yes(egl_surface.is_some()),
            yes(egl_surface.is_some_and(|s| s.as_ptr() as usize as i64 == surface))
        ));
        let context = GlContext::new(&w);
        t(format!("context: {}", result(&context)));
        let context = context.expect("a context");
        t(format!("current: {}", yes(context.is_current())));
        t(format!("GL_VERSION: {}", gl_string(0x1F02)));
        t(format!("GL_RENDERER: {}", gl_string(0x1F01)));
        t(format!("GL_VENDOR: {}", gl_string(0x1F00)));
        t(format!(
            "GL_SHADING_LANGUAGE_VERSION: {}",
            gl_string(0x8B8C)
        ));
        t(format!(
            "attributes: red={} green={} blue={} alpha={} depth={} stencil={} doublebuffer={} version={}.{} profile={}",
            attr(GlAttr::RedSize),
            attr(GlAttr::GreenSize),
            attr(GlAttr::BlueSize),
            attr(GlAttr::AlphaSize),
            attr(GlAttr::DepthSize),
            attr(GlAttr::StencilSize),
            attr(GlAttr::DoubleBuffer),
            attr(GlAttr::ContextMajorVersion),
            attr(GlAttr::ContextMinorVersion),
            attr(GlAttr::ContextProfileMask)
        ));
        let interval = |label: &str| {
            t(format!(
                "{label}: {}",
                gl::gl_get_swap_interval()
                    .map_or_else(|e| format!("error: {e}"), |v| v.to_string())
            ));
        };
        interval("swap interval");
        t(format!(
            "set swap interval 1: {}",
            result(&gl::gl_set_swap_interval(1))
        ));
        interval("swap interval");
        t(format!(
            "set swap interval -1: {}",
            result(&gl::gl_set_swap_interval(-1))
        ));
        t(format!(
            "set swap interval 0: {}",
            result(&gl::gl_set_swap_interval(0))
        ));
        let p = clear_and_read(1.0, 0.0, 1.0);
        t(format!("clear: {},{},{},{}", p[0], p[1], p[2], p[3]));
        t(format!("swap: {}", result(&gl::gl_swap_window(&w))));
        drop(context);
        t("context destroyed");

        // The OpenGL ES 2 renderer
        let r = Renderer::for_window(&w, Some("opengles2"));
        t(format!("renderer: {}", result(&r)));
        let mut r = r.expect("the renderer");
        ev::pump_for(300);
        ev::take();
        renderer_state("renderer", &mut r, &w);
        scenes(&mut r, "opengles2");
        check_custom_blend(&mut r);
        for vsync in [1, 0] {
            t(format!(
                "set vsync {vsync}: {}, vsync={}",
                result(&r.set_vsync(vsync)),
                r.vsync()
            ));
        }

        // A trip to the background and back: the context kept and restored
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
        ev::unwatch();
        init::quit();
        t("quit");

        let mismatches = MISMATCHES.lock().unwrap().clone();
        for m in &mismatches {
            println!("android_gles: against the software renderer: {m}");
        }
        let trace = TRACE.lock().unwrap().clone();
        check_trace(&trace, EXPECTED);
        assert!(
            mismatches.is_empty(),
            "the renderer's pixels differ from the software renderer's: {mismatches:#?}"
        );
    }
}
