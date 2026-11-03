// Tests of the OpenGL ES 1 renderer on the android video driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, whose main
//! function traces, against upstream's C SDL:
//!
//! * the OpenGL ES 1 renderer ("opengles") on a window made without a
//!   graphics API, which the renderer makes again as an OpenGL ES 1.1
//!   window: the window's OpenGL flag and the attributes as SDL reads them
//!   back, the GL strings of the renderer's context (`libGLESv1_CM.so`,
//!   the emulator's translator over SwiftShader) and the extensions the
//!   renderer looks for, its name, output size, maximum texture size,
//!   texture formats (RGBA32 only) and vsync;
//! * the custom blend modes it takes (`GLES_SupportsBlendMode()`, from
//!   those extensions); the scenes the other renderers' tests draw
//!   (`uikit_support::scenes`, each read back and hashed, and compared
//!   with the software renderer, within the tolerances the other
//!   renderers' tests allow: the other formats, YUV ones too, through
//!   the front end's RGBA32 textures), partial reads, and the custom blend
//!   mode's pixels where it is taken; the emulator's OpenGL ES 1
//!   translator fails the `GL_OES_framebuffer_object` functions (which
//!   upstream takes as there on Android: the `FIXME (upstream)` in the
//!   renderer), so there are no render targets (their scene's error is
//!   traced, and the "target read" is the window's);
//! * a scene drawn while the application's own context on the window is
//!   current (the renderer makes its own current to draw), and vsync 1 and
//!   0;
//! * a trip to the background: the context is kept (the activity's
//!   surface is made again, and the context made current with it), with
//!   the application's and render events and a scene drawn again;
//! * a renderer asked for linear output (upstream's renderer doesn't check
//!   the colorspace: it is made, and draws as for sRGB).
//!
//! The expected trace is upstream's C SDL's, from the same program in C on
//! the same emulator (API 35, Google APIs, x86-64): upstream's CMake build
//! has the renderer on Android. It is built with the Rust renderer's two
//! fixes of upstream's (the framebuffer object functions tried before
//! they're taken, without which upstream's crashes as it's destroyed, and
//! the textures' address modes set when they're made). Elsewhere this
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
    use std::ffi::{c_char, CStr};

    use sdl3::events::window::WindowFlags;
    use sdl3::events::EventType;
    use sdl3::init::{self, InitFlags};
    use sdl3::render::{Renderer, RendererCreateInfo};
    use sdl3::video::gl::{self, GlAttr, GlContext};
    use sdl3::video::{self, BlendFactor, BlendMode, BlendOperation, Colorspace, Window};

    use super::android_support::check_trace;
    use super::android_support::events::{self as ev, is_app_event};
    use super::scenes::{check_custom_blend, clear_and_fill, scene, scenes, t, MISMATCHES, TRACE};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
init: ok
video driver: android
defaults: profile=4 version=2.0
window: ok
window: opengl: no
renderer: ok
renderer: window: opengl: yes, attributes: profile=4 version=1.1
context: current: yes
GL_VERSION: OpenGL ES-CM 1.1 (OpenGL ES 3.0 SwiftShader 4.0.0.1)
GL_RENDERER: Android Emulator OpenGL ES Translator (Google SwiftShader)
GL_VENDOR: Google (Google Inc.)
extension GL_OES_framebuffer_object: no
extension GL_OES_blend_func_separate: yes
extension GL_OES_blend_equation_separate: yes
extension GL_OES_blend_subtract: yes
extension GL_EXT_blend_minmax: no
renderer: name=opengles output=2400x1080 pixels=2400x1080 max_texture_size=8192 vsync=0
renderer: formats: SDL_PIXELFORMAT_ABGR8888
blend mode add: ok
blend mode rev_subtract: ok
blend mode minimum: error: That operation is not supported
blend mode maximum: error: That operation is not supported
blend mode separate_factors: ok
blend mode separate_operations: ok
scene clear and fill: 64x48 5fe1f24a
scene points and lines: 64x48 75192d12
scene viewport and clip: 64x48 b09f79e5
scene blend modes: 64x48 ebb10095
scene texture SDL_PIXELFORMAT_ARGB8888: 64x48 34d06d0b
scene texture SDL_PIXELFORMAT_ABGR8888: 64x48 34d06d0b
scene texture SDL_PIXELFORMAT_RGB24: 64x48 4fb014c5
scene texture SDL_PIXELFORMAT_XRGB8888: 64x48 4fb014c5
scene streaming texture: 64x48 c2319ec5
scene render target: error: GL_OES_framebuffer_object not supported
scene render target: 64x48 c2319ec5
scene packed SDL_PIXELFORMAT_ABGR2101010: 64x48 8bdc6af5
scene packed SDL_PIXELFORMAT_RGBA64_FLOAT: 64x48 9a74c64d
scene packed SDL_PIXELFORMAT_RGBA128_FLOAT: 64x48 9a74c64d
scene palette texture: 64x48 65a75445
scene yuv SDL_PIXELFORMAT_IYUV: 64x48 e59dfe55
scene yuv SDL_PIXELFORMAT_YV12: 64x48 e59dfe55
scene yuv SDL_PIXELFORMAT_NV12: 64x48 e59dfe55
scene yuv SDL_PIXELFORMAT_NV21: 64x48 e59dfe55
scene IYUV planes: 64x48 e59dfe55
scene NV12 planes: 64x48 e59dfe55
scene locked texture: 64x48 76d14245
scene locked NV12 texture: 64x48 e59dfe55
scene custom blend mode: 64x48 af879425
scene wrapping geometry: 64x48 dd468f45
scene colored geometry: 64x48 fc2fd151
scene linear scaling: 64x48 5599895b
scene debug text: 64x48 1ecf4704
partial read: 8x8 f24b7925
target read: 2400x1080 f98adfc5
application's context: ok
application's context: current: yes
scene with the application's context: 64x48 5fe1f24a
after the scene: the application's context current: no
application's context made current: ok
application's context destroyed
scene after the application's context: 64x48 5fe1f24a
set vsync 1: ok, vsync=1
set vsync 0: ok, vsync=0
trip: app events: WILL_ENTER_BACKGROUND(0,0), DID_ENTER_BACKGROUND(0,0), WILL_ENTER_FOREGROUND(0,0), DID_ENTER_FOREGROUND(0,0)
trip: render events: (none)
after the trip: name=opengles output=2400x1080 pixels=2400x1080 max_texture_size=8192 vsync=0
after the trip: formats: SDL_PIXELFORMAT_ABGR8888
scene after the trip: 64x48 5fe1f24a
renderer destroyed
linear renderer: ok
linear renderer: name=opengles output colorspace=301991168
linear renderer destroyed
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

    fn attr(a: GlAttr) -> String {
        gl::gl_get_attribute(a).map_or_else(|e| format!("error: {e}"), |v| v.to_string())
    }

    /// `glGetString(name)` of the current context.
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

    /// The custom blend modes, each with whether the renderer takes it.
    fn blend_modes(r: &mut Renderer) {
        use BlendFactor as F;
        use BlendOperation as O;
        let modes = [
            ("add", F::One, F::One, O::Add, F::One, F::One, O::Add),
            (
                "rev_subtract",
                F::One,
                F::One,
                O::RevSubtract,
                F::One,
                F::One,
                O::RevSubtract,
            ),
            (
                "minimum",
                F::One,
                F::One,
                O::Minimum,
                F::One,
                F::One,
                O::Minimum,
            ),
            (
                "maximum",
                F::One,
                F::One,
                O::Maximum,
                F::One,
                F::One,
                O::Maximum,
            ),
            (
                "separate_factors",
                F::SrcAlpha,
                F::OneMinusSrcAlpha,
                O::Add,
                F::Zero,
                F::One,
                O::Add,
            ),
            (
                "separate_operations",
                F::One,
                F::One,
                O::RevSubtract,
                F::Zero,
                F::One,
                O::Add,
            ),
        ];
        for (name, sc, dc, co, sa, da, ao) in modes {
            let mode = BlendMode::compose_custom(sc, dc, co, sa, da, ao);
            t(format!(
                "blend mode {name}: {}",
                result(&r.set_draw_blend_mode(mode))
            ));
        }
        let _ = r.set_draw_blend_mode(BlendMode::NONE);
    }

    /// Whether the renderer takes the scenes' custom blend mode.
    fn takes_custom_blend(r: &mut Renderer) -> bool {
        let mode = BlendMode::compose_custom(
            BlendFactor::One,
            BlendFactor::One,
            BlendOperation::RevSubtract,
            BlendFactor::Zero,
            BlendFactor::One,
            BlendOperation::Add,
        );
        let taken = r.set_draw_blend_mode(mode).is_ok();
        let _ = r.set_draw_blend_mode(BlendMode::NONE);
        taken
    }

    pub(super) fn main() {
        t(format!("init: {}", result(&init::init(InitFlags::VIDEO))));
        ev::watch();
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap_or("(none)")
        ));
        t(format!(
            "defaults: profile={} version={}.{}",
            attr(GlAttr::ContextProfileMask),
            attr(GlAttr::ContextMajorVersion),
            attr(GlAttr::ContextMinorVersion)
        ));

        // A window without a graphics API, landscape
        let created = Window::create("android gles1", 320, 240, WindowFlags::default());
        t(format!("window: {}", result(&created)));
        let w = created.expect("the window");
        ev::pump_until(|| w.size_in_pixels().is_ok_and(|(w, h)| w > h), 5000, 300);
        ev::take();
        let opengl = |w: &Window| yes(w.flags().is_ok_and(|f| f.contains(WindowFlags::OPENGL)));
        t(format!("window: opengl: {}", opengl(&w)));

        // The OpenGL ES 1 renderer, which makes the window an OpenGL ES 1.1
        // one
        let r = Renderer::for_window(&w, Some("opengles"));
        t(format!("renderer: {}", result(&r)));
        let mut r = r.expect("the renderer");
        ev::pump_until(|| w.size_in_pixels().is_ok_and(|(w, h)| w > h), 5000, 300);
        ev::take();
        t(format!(
            "renderer: window: opengl: {}, attributes: profile={} version={}.{}",
            opengl(&w),
            attr(GlAttr::ContextProfileMask),
            attr(GlAttr::ContextMajorVersion),
            attr(GlAttr::ContextMinorVersion)
        ));
        // (a draw makes the renderer's context current)
        r.set_draw_color(0, 0, 0, 255);
        let _ = r.clear();
        let _ = r.flush();
        t(format!(
            "context: current: {}",
            yes(gl::gl_current_context().is_ok_and(|c| c.is_some()))
        ));
        t(format!("GL_VERSION: {}", gl_string(0x1F02)));
        t(format!("GL_RENDERER: {}", gl_string(0x1F01)));
        t(format!("GL_VENDOR: {}", gl_string(0x1F00)));
        for extension in [
            "GL_OES_framebuffer_object",
            "GL_OES_blend_func_separate",
            "GL_OES_blend_equation_separate",
            "GL_OES_blend_subtract",
            "GL_EXT_blend_minmax",
        ] {
            t(format!(
                "extension {extension}: {}",
                yes(gl::gl_extension_supported(extension))
            ));
        }
        renderer_state("renderer", &mut r, &w);
        blend_modes(&mut r);
        scenes(&mut r, "opengles");
        if takes_custom_blend(&mut r) {
            check_custom_blend(&mut r);
        }

        // The application's own context on the window, current while the
        // renderer draws
        let context = GlContext::new(&w);
        t(format!("application's context: {}", result(&context)));
        let context = context.expect("the application's context");
        t(format!(
            "application's context: current: {}",
            yes(context.is_current())
        ));
        scene(
            &mut r,
            "with the application's context",
            Some((0, 0)),
            &clear_and_fill,
        );
        t(format!(
            "after the scene: the application's context current: {}",
            yes(context.is_current())
        ));
        t(format!(
            "application's context made current: {}",
            result(&context.make_current(Some(&w)))
        ));
        drop(context);
        t("application's context destroyed");
        scene(
            &mut r,
            "after the application's context",
            Some((0, 0)),
            &clear_and_fill,
        );

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

        // A renderer asked for linear output
        let info = RendererCreateInfo {
            name: Some("opengles".to_owned()),
            output_colorspace: Some(Colorspace::SRGB_LINEAR),
            ..Default::default()
        };
        let r = Renderer::for_window_with(&w, &info);
        t(format!("linear renderer: {}", result(&r)));
        if let Ok(r) = r {
            t(format!(
                "linear renderer: name={} output colorspace={}",
                r.name(),
                r.properties()
                    .get_number(sdl3::render::PROP_RENDERER_OUTPUT_COLORSPACE_NUMBER)
                    .unwrap_or(-1)
            ));
            drop(r);
            t("linear renderer destroyed");
        }

        w.destroy();
        ev::pump_for(200);
        ev::take();
        ev::unwatch();
        init::quit();
        t("quit");

        let mismatches = MISMATCHES.lock().unwrap().clone();
        for m in &mismatches {
            println!("android_gles1: against the software renderer: {m}");
        }
        let trace = TRACE.lock().unwrap().clone();
        check_trace(&trace, EXPECTED);
        assert!(
            mismatches.is_empty(),
            "the renderer's pixels differ from the software renderer's: {mismatches:#?}"
        );
    }
}
