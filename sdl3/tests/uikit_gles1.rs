// Tests of the OpenGL ES 1 renderer on the UIKit video driver.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver only starts in an app, so this is a program of
//! its own (`harness = false`), run as an app on the iOS and tvOS
//! Simulators by `tools/ios_run.py` (the `__DATA,__sdl3_runner` section
//! below), as the other `uikit_*` programs are. Its main function traces:
//!
//! * the OpenGL ES 1 renderer ("opengles") on window D, which asks for no
//!   graphics API and gets the OpenGL flag (the renderer makes it again
//!   for OpenGL ES 1.1, the attributes it asks for): the attributes as SDL
//!   reads them back, the GL strings of its EAGL context
//!   (`kEAGLRenderingAPIOpenGLES1`) and the extensions it looks for, its
//!   name, output size, maximum texture size, texture formats (RGBA32
//!   only) and vsync (UIKit sets no swap interval);
//! * the custom blend modes it takes (`GLES_SupportsBlendMode()`); the
//!   scenes the other renderers' tests draw (`uikit_support::scenes`, each
//!   read back and hashed, and compared with the software renderer:
//!   the other formats, YUV ones too, through the front end's RGBA32
//!   textures), partial reads, and the custom blend mode's pixels where it
//!   is taken;
//! * a scene drawn while the application's own context on the window is
//!   current (the renderer makes its own current to draw);
//! * the renderer on Metal window M, which is made again with the OpenGL
//!   flag (UIKit can't reconfigure a window), and one asked for linear
//!   output there (upstream's renderer doesn't check the colorspace: it is
//!   made, and draws as for sRGB).
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulators (an iPhone 16 with iOS 18.5,
//! an Apple TV 4K (3rd generation) with tvOS 18.5): upstream's CMake build
//! has the renderer with OpenGL ES on the UIKit platforms (on tvOS with
//! OpenGL ES allowed, as the Xcode project's `SDL_build_config_ios.h`
//! builds it), with the Rust renderer's two fixes of upstream's (the
//! textures' address modes set when they're made, without which a linearly
//! scaled texture's edges blend with the opposite ones, and the framebuffer
//! object functions tried before they're taken where the extension isn't
//! reported, which EAGL reports). If OpenGL ES 1 is missing (Apple deprecated OpenGL ES), the
//! program skips (capability `gles`), unless `SDL3_TEST_REQUIRE` names it.
//! Elsewhere this program does nothing.

#[cfg(not(any(target_os = "ios", target_os = "tvos")))]
fn main() {}

/// The runner's words: run as an app.
#[cfg(any(target_os = "ios", target_os = "tvos"))]
#[used]
#[link_section = "__DATA,__sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(any(target_os = "ios", target_os = "tvos"))]
mod uikit_support;

#[cfg(any(target_os = "ios", target_os = "tvos"))]
fn main() {
    ios::main()
}

#[cfg(any(target_os = "ios", target_os = "tvos"))]
mod ios {
    use std::ffi::{c_char, CStr};
    use std::time::{Duration, Instant};

    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event};
    use sdl3::init::{self, InitFlags};
    use sdl3::render::{Renderer, RendererCreateInfo};
    use sdl3::video::gl::{self, GlAttr, GlContext};
    use sdl3::video::{self, BlendFactor, BlendMode, BlendOperation, Colorspace, Window};

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::scenes::{
        check_custom_blend, clear_and_fill, scene, scenes, t, MISMATCHES, TRACE,
    };
    use super::uikit_support::{check_trace, checked, elapsed, exit};

    /// Upstream's trace (the C program, on an iPhone 16 simulator with
    /// iOS 18.5).
    #[cfg(target_os = "ios")]
    const EXPECTED: &str = "\
init video: ok
video driver: uikit
application state: 0
defaults: profile=4 version=2.0
D: create 320x480: ok
D: opengl: yes
D: create opengles renderer: ok
D: renderer: opengl: yes, attributes: profile=4 version=1.1
D: context: current: yes, EAGL API: 1
D: GL_VERSION: OpenGL ES-CM 1.1 APPLE-21.1.1
D: GL_RENDERER: Apple Software Renderer
D: GL_VENDOR: Apple Inc.
D: extension GL_OES_framebuffer_object: yes
D: extension GL_OES_blend_func_separate: yes
D: extension GL_OES_blend_equation_separate: yes
D: extension GL_OES_blend_subtract: yes
D: extension GL_EXT_blend_minmax: yes
D: renderer: name=opengles output=393x852 pixels=393x852 max_texture_size=4096 vsync=0
D: renderer: formats: SDL_PIXELFORMAT_ABGR8888
D: blend mode add: ok
D: blend mode rev_subtract: ok
D: blend mode minimum: ok
D: blend mode maximum: ok
D: blend mode separate_factors: ok
D: blend mode separate_operations: ok
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
scene colored geometry: 64x48 7a2c48ad
scene linear scaling: 64x48 5599895b
scene debug text: 64x48 1ecf4704
partial read: 8x8 f24b7925
target read: 20x10 da6561f5
D: application's context: ok
D: application's context: current: yes
scene with the application's context: 64x48 5fe1f24a
D: after the scene: the application's context current: no
D: application's context destroyed
scene after the application's context: 64x48 5fe1f24a
D: set vsync 1: ok, vsync=1
D: set vsync 0: ok, vsync=0
D: render events: (none)
D: renderer destroyed
D: destroyed
M: create 320x480 metal: ok
M: create opengles renderer: ok
M: renderer: opengl: yes, metal: no
M: renderer: name=opengles output=393x852 pixels=393x852 max_texture_size=4096 vsync=0
M: renderer: formats: SDL_PIXELFORMAT_ABGR8888
scene M: 64x48 5fe1f24a
M: renderer destroyed
M: linear renderer: ok
M: linear renderer: name=opengles output colorspace=301991168
M: linear renderer destroyed
M: destroyed
quit
";

    /// Upstream's trace (the C program, on an Apple TV 4K (3rd generation)
    /// simulator with tvOS 18.5).
    #[cfg(target_os = "tvos")]
    const EXPECTED: &str = "\
init video: ok
video driver: uikit
application state: 0
defaults: profile=4 version=2.0
D: create 320x480: ok
D: opengl: yes
D: create opengles renderer: ok
D: renderer: opengl: yes, attributes: profile=4 version=1.1
D: context: current: yes, EAGL API: 1
D: GL_VERSION: OpenGL ES-CM 1.1 APPLE-21.1.1
D: GL_RENDERER: Apple Software Renderer
D: GL_VENDOR: Apple Inc.
D: extension GL_OES_framebuffer_object: yes
D: extension GL_OES_blend_func_separate: yes
D: extension GL_OES_blend_equation_separate: yes
D: extension GL_OES_blend_subtract: yes
D: extension GL_EXT_blend_minmax: yes
D: renderer: name=opengles output=1920x1080 pixels=1920x1080 max_texture_size=4096 vsync=0
D: renderer: formats: SDL_PIXELFORMAT_ABGR8888
D: blend mode add: ok
D: blend mode rev_subtract: ok
D: blend mode minimum: ok
D: blend mode maximum: ok
D: blend mode separate_factors: ok
D: blend mode separate_operations: ok
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
scene colored geometry: 64x48 7a2c48ad
scene linear scaling: 64x48 5599895b
scene debug text: 64x48 1ecf4704
partial read: 8x8 f24b7925
target read: 20x10 da6561f5
D: application's context: ok
D: application's context: current: yes
scene with the application's context: 64x48 5fe1f24a
D: after the scene: the application's context current: no
D: application's context destroyed
scene after the application's context: 64x48 5fe1f24a
D: set vsync 1: ok, vsync=1
D: set vsync 0: ok, vsync=0
D: render events: (none)
D: renderer destroyed
D: destroyed
M: create 320x480 metal: ok
M: create opengles renderer: ok
M: renderer: opengl: yes, metal: no
M: renderer: name=opengles output=1920x1080 pixels=1920x1080 max_texture_size=4096 vsync=0
M: renderer: formats: SDL_PIXELFORMAT_ABGR8888
scene M: 64x48 5fe1f24a
M: renderer destroyed
M: linear renderer: ok
M: linear renderer: name=opengles output colorspace=301991168
M: linear renderer destroyed
M: destroyed
quit
";

    fn yes(b: bool) -> &'static str {
        if b {
            "yes"
        } else {
            "no"
        }
    }

    fn ok<T>(r: &sdl3::Result<T>) -> String {
        match r {
            Ok(_) => "ok".into(),
            Err(e) => format!("error: {e}"),
        }
    }

    fn attr(a: GlAttr) -> String {
        gl::gl_get_attribute(a).map_or_else(|e| format!("error: {e}"), |v| v.to_string())
    }

    /// Pump the events for `ms` milliseconds (10 ms steps), dropping them.
    fn settle(ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        loop {
            events::pump();
            while let Some(_event) = events::poll() {}
            if Instant::now() >= end {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Pump until a render event came (or `ms` milliseconds went): the
    /// render events' names.
    fn render_events(ms: u64) -> String {
        let mut names = Vec::new();
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            events::pump();
            while let Some(event) = events::poll() {
                if let Event::Render(r) = &event {
                    names.push(super::uikit_support::event_name(r.event_type));
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if names.is_empty() {
            "(none)".into()
        } else {
            names.join(", ")
        }
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

    /// EAGL's current context.
    fn eagl_current() -> Id {
        // SAFETY: EAGLContext's class method.
        unsafe { objc::send0(objc::class("EAGLContext"), "currentContext") }
    }

    /// The API of EAGL's current context (0 without one).
    fn eagl_api() -> usize {
        let c = eagl_current();
        if c.is_null() {
            return 0;
        }
        // SAFETY: an EAGLContext property.
        unsafe { objc::send0(c, "API") }
    }

    fn opengl(w: &Window) -> &'static str {
        yes(w.flags().is_ok_and(|f| f.contains(WindowFlags::OPENGL)))
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
                "D: blend mode {name}: {}",
                ok(&r.set_draw_blend_mode(mode))
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
        println!("uikit_gles1: run_app returned {status}, which it never does");
        exit(1);
    }

    /// Whether the simulator has OpenGL ES 1 (else the program skips,
    /// unless `SDL3_TEST_REQUIRE` names `gles`).
    fn gles_available() -> bool {
        let c = objc::class("EAGLContext");
        let why = if c.is_null() {
            "no EAGLContext class".to_string()
        } else {
            // SAFETY: EAGLContext's alloc and initializer, released.
            let context: Id = unsafe {
                let context: Id = objc::send0(c, "alloc");
                objc::send1(context, "initWithAPI:", 1usize)
            };
            if context.is_null() {
                "EAGL couldn't make an OpenGL ES 1 context".to_string()
            } else {
                // SAFETY: the context made above.
                unsafe {
                    let () = objc::send0(context, "release");
                }
                return true;
            }
        };
        let required = std::env::var("SDL3_TEST_REQUIRE").unwrap_or_default();
        if required
            .split(',')
            .map(str::trim)
            .any(|c| c == "gles" || c == "all")
        {
            panic!("required capability gles unavailable: {why}");
        }
        eprintln!("note: skipping, gles unavailable: {why}");
        false
    }

    fn main_function() {
        println!("uikit_gles1: {:.3} s: main", elapsed());
        if !gles_available() {
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
        let end = Instant::now() + Duration::from_secs(30);
        while application_state() != 0 && Instant::now() < end {
            settle(10);
        }
        settle(100);
        t(format!("application state: {}", application_state()));
        t(format!(
            "defaults: profile={} version={}.{}",
            attr(GlAttr::ContextProfileMask),
            attr(GlAttr::ContextMajorVersion),
            attr(GlAttr::ContextMinorVersion)
        ));

        // D: a window asking for no graphics API (the OpenGL flag)
        let created = Window::create("D", 320, 480, WindowFlags::NONE);
        t(format!("D: create 320x480: {}", ok(&created)));
        let d = created.unwrap();
        settle(500);
        t(format!("D: opengl: {}", opengl(&d)));

        let r = Renderer::for_window(&d, Some("opengles"));
        t(format!("D: create opengles renderer: {}", ok(&r)));
        let mut r = r.unwrap();
        settle(500);
        t(format!(
            "D: renderer: opengl: {}, attributes: profile={} version={}.{}",
            opengl(&d),
            attr(GlAttr::ContextProfileMask),
            attr(GlAttr::ContextMajorVersion),
            attr(GlAttr::ContextMinorVersion)
        ));
        // (a draw makes the renderer's context current)
        r.set_draw_color(0, 0, 0, 255);
        let _ = r.clear();
        let _ = r.flush();
        t(format!(
            "D: context: current: {}, EAGL API: {}",
            yes(gl::gl_current_context().is_ok_and(|c| c.is_some())),
            eagl_api()
        ));
        t(format!("D: GL_VERSION: {}", gl_string(0x1F02)));
        t(format!("D: GL_RENDERER: {}", gl_string(0x1F01)));
        t(format!("D: GL_VENDOR: {}", gl_string(0x1F00)));
        for extension in [
            "GL_OES_framebuffer_object",
            "GL_OES_blend_func_separate",
            "GL_OES_blend_equation_separate",
            "GL_OES_blend_subtract",
            "GL_EXT_blend_minmax",
        ] {
            t(format!(
                "D: extension {extension}: {}",
                yes(gl::gl_extension_supported(extension))
            ));
        }
        renderer_state("D: renderer", &mut r, &d);
        blend_modes(&mut r);
        scenes(&mut r, "opengles");
        if takes_custom_blend(&mut r) {
            check_custom_blend(&mut r);
        }

        // The application's own context on the window, current while the
        // renderer draws
        let context = GlContext::new(&d);
        t(format!("D: application's context: {}", ok(&context)));
        let context = context.unwrap();
        t(format!(
            "D: application's context: current: {}",
            yes(context.is_current())
        ));
        scene(
            &mut r,
            "with the application's context",
            Some((0, 0)),
            &clear_and_fill,
        );
        t(format!(
            "D: after the scene: the application's context current: {}",
            yes(context.is_current())
        ));
        drop(context);
        t("D: application's context destroyed");
        scene(
            &mut r,
            "after the application's context",
            Some((0, 0)),
            &clear_and_fill,
        );

        for vsync in [1, 0] {
            t(format!(
                "D: set vsync {vsync}: {}, vsync={}",
                ok(&r.set_vsync(vsync)),
                r.vsync()
            ));
        }
        t(format!("D: render events: {}", render_events(300)));
        drop(r);
        t("D: renderer destroyed");
        d.destroy();
        t("D: destroyed");
        settle(500);

        // M: a Metal window, made again with the OpenGL flag
        let created = Window::create("M", 320, 480, WindowFlags::METAL);
        t(format!("M: create 320x480 metal: {}", ok(&created)));
        let m = created.unwrap();
        settle(500);
        let r = Renderer::for_window(&m, Some("opengles"));
        t(format!("M: create opengles renderer: {}", ok(&r)));
        let mut r = r.unwrap();
        settle(500);
        t(format!(
            "M: renderer: opengl: {}, metal: {}",
            opengl(&m),
            yes(m.flags().is_ok_and(|f| f.contains(WindowFlags::METAL)))
        ));
        renderer_state("M: renderer", &mut r, &m);
        scene(&mut r, "M", Some((0, 0)), &clear_and_fill);
        drop(r);
        t("M: renderer destroyed");

        // A renderer asked for linear output
        let info = RendererCreateInfo {
            name: Some("opengles".to_owned()),
            output_colorspace: Some(Colorspace::SRGB_LINEAR),
            ..Default::default()
        };
        let r = Renderer::for_window_with(&m, &info);
        t(format!("M: linear renderer: {}", ok(&r)));
        if let Ok(r) = r {
            t(format!(
                "M: linear renderer: name={} output colorspace={}",
                r.name(),
                r.properties()
                    .get_number(sdl3::render::PROP_RENDERER_OUTPUT_COLORSPACE_NUMBER)
                    .unwrap_or(-1)
            ));
            drop(r);
            t("M: linear renderer destroyed");
        }
        m.destroy();
        t("M: destroyed");
        settle(300);

        init::quit();
        t("quit");

        let mismatches = MISMATCHES.lock().unwrap().clone();
        for m in &mismatches {
            println!("uikit_gles1: against the software renderer: {m}");
        }
        check_trace(&TRACE.lock().unwrap(), EXPECTED);
        assert!(
            mismatches.is_empty(),
            "the OpenGL ES 1 renderer against the software renderer:\n{}",
            mismatches.join("\n")
        );
    }
}
