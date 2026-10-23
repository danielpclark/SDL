// Tests of the Emscripten video driver's WebGL contexts and the OpenGL ES 2
// renderer in a real browser.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The WebGL contexts of the canvas video driver
//! (`SDL_emscriptenopengles.c`) and the OpenGL ES 2 renderer on them
//! (`SDL_render_gles2.c` with `USE_VERTEX_BUFFER_OBJECTS`) in headless
//! Chromium (WebGL on SwiftShader): a program of its own (`harness =
//! false`) that asks `tools/emscripten_run.py` for the browser, as
//! `emscripten_video.rs` does. It's linked with `-sMAX_WEBGL_VERSION=2`,
//! as every test program is (`.cargo/config.toml`).
//!
//! Each step writes a trace: what SDL answered (contexts, attributes,
//! errors, swap intervals and emscripten's main loop timing), what the
//! GL functions SDL looks up do (`glClear()` and `glReadPixels()` through
//! `SDL_GL_GetProcAddress()`), what the context is (`GL.currentContext`),
//! and for the renderer's scenes the pixels read back (an FNV-1a hash of
//! them and a few of them). [`EXPECTED`] is the trace of upstream's C
//! (`libSDL3.a`, SDL 3.5.0) on the same page with the same steps, and each
//! step's part must be the same here. The steps:
//!
//! * `init`: the driver, and the attributes before any context (OpenGL ES
//!   2.0 by default: upstream's Emscripten build has OpenGL ES 2 only);
//! * `es2`: an OpenGL window's WebGL 1 context (an OpenGL ES 2.0 one), made
//!   current, its attributes, the GL functions there are, a clear read
//!   back, a second context (refused), swapping, the swap interval before a
//!   main loop runs (kept for the first event pump), a context made current
//!   with another window (refused), released, destroyed and made again;
//! * `es3`: an OpenGL ES 3.0 context, WebGL 2 (`GL_MAJOR_VERSION`;
//!   emscripten's `glGetString()` keeps the first context's strings for
//!   every context, which the trace shows);
//! * `attrs`: the attributes WebGL takes (alpha, depth, stencil,
//!   antialiasing) and what the context has;
//! * `renderer`: the OpenGL ES 2 renderer on a window that isn't an OpenGL
//!   one yet (SDL makes it one), its texture formats, and its scenes: a
//!   clear, shapes (rectangles, points, lines), blend modes, a texture
//!   (scaled, color and alpha modulated, rotated and flipped), a streaming
//!   texture, IYUV and NV12 textures, a render target (read back while it's
//!   the target, then drawn), and geometry (colored and textured); then
//!   vsync (the pending interval) and a present;
//! * `renderer_es3`: the same scenes on an OpenGL ES 3.0 window (WebGL 2;
//!   here the renderer's `OPENGLES_300` shaders, upstream's C its OpenGL ES
//!   2.0 ones, with the same pixels), and `renderer_es3_shaders` (not in
//!   the C trace) that they are GLSL ES 3.00;
//! * `extensions`: `SDL_GL_ExtensionSupported()` for every extension the
//!   context lists, and the hint that hides one (not in the C trace);
//!
//! then, in the main callbacks: `enter` (an interval set before the main
//! loop), `iter1` (the first event pump sets it: the main loop's timing),
//! `iter2` (with `SDL_HINT_MAIN_CALLBACK_RATE`, which then decides the
//! timing), `iter3` (the hint reset, and the renderer's present on the
//! page's canvas, copied by a 2D canvas in the same task), `loss` (the
//! context lost through `WEBGL_lose_context`: what SDL and GL do then, and
//! once the page prevents `webglcontextlost`'s default, the context
//! restored), `quit`.
//!
//! In the attribute lines two answers are the repository's, marked Note
//! (upstream) in `sdl3::video::gl`: `SDL_GL_CONTEXT_RESET_NOTIFICATION` is
//! the value set (upstream: "Unknown OpenGL attribute") and
//! `SDL_GL_FLOATBUFFERS` without `GL_ARB_color_buffer_float` is
//! "unsupported" (upstream: an empty error).
//!
//! Each step prints a libtest-like `test NAME ... ok` line (with both
//! traces when they differ); under node everything skips as `browser`, and
//! without WebGL as `webgl`. `SDL3_EMSCRIPTEN_BROWSER=1
//! SDL3_TEST_REQUIRE=browser,webgl cargo test -p sdl3 --target
//! wasm32-unknown-emscripten --test emscripten_gl` runs it as CI does.

#[cfg(not(target_os = "emscripten"))]
fn main() {}

#[cfg(target_os = "emscripten")]
fn main() -> std::process::ExitCode {
    webgl::main()
}

/// The program's request for the browser (see `emscripten_browser.rs`).
#[cfg(target_os = "emscripten")]
#[used]
#[link_section = "sdl3_runner"]
static RUNNER: [u8; 7] = *b"browser";

#[cfg(target_os = "emscripten")]
mod webgl {
    use std::ffi::{c_char, c_int, c_void, CStr};
    use std::fmt::Write as _;
    use std::panic::{self, AssertUnwindSafe};
    use std::process::ExitCode;
    use std::sync::Mutex;

    use sdl3::app::{self, AppCallbacks, AppResult};
    use sdl3::events::window::WindowFlags;
    use sdl3::events::Event;
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::properties::Properties;
    use sdl3::render::{
        Renderer, Texture, TextureAccess, Vertex, PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER,
    };
    use sdl3::video::gl::{self, GlAttr, GlContext};
    use sdl3::video::pixels::{FColor, PixelFormat};
    use sdl3::video::rect::{FPoint, FRect};
    use sdl3::video::surface::ScaleMode;
    use sdl3::video::window::{
        PROP_WINDOW_CREATE_EMSCRIPTEN_CANVAS_ID_STRING, PROP_WINDOW_CREATE_FLAGS_NUMBER,
        PROP_WINDOW_CREATE_HEIGHT_NUMBER, PROP_WINDOW_CREATE_TITLE_STRING,
        PROP_WINDOW_CREATE_WIDTH_NUMBER,
    };
    use sdl3::video::{self as sdl_video, BlendMode, FlipMode, Window};
    use sdl3_emscripten::{em_asm, em_asm_int, sys};

    fn required(capability: &str) -> bool {
        std::env::var("SDL3_TEST_REQUIRE")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .any(|c| c == "all" || c == capability)
    }

    fn in_browser() -> bool {
        // SAFETY: plain JavaScript.
        unsafe {
            em_asm_int!(
                r#"
                var node = typeof process === 'object' && !!(process.versions && process.versions.node);
                return !node && typeof window === 'object' && typeof document === 'object' && typeof navigator === 'object' ? 1 : 0;
            "#
            ) != 0
        }
    }

    fn has_webgl() -> bool {
        // SAFETY: plain JavaScript.
        unsafe {
            em_asm_int!(
                "try { return document.createElement('canvas').getContext('webgl') ? 1 : 0; } catch (e) { return 0; }"
            ) != 0
        }
    }

    // -- JavaScript helpers ----------------------------------------------------

    /// The value of the JavaScript expression `expr` (evaluated in the
    /// glue's scope: `Module`, `GL`, …), as a string.
    fn js(expr: &str) -> String {
        let code = format!("{expr}\0");
        // SAFETY: plain JavaScript; the code is NUL-terminated.
        let len = unsafe {
            em_asm_int!(
                r#"
                var v = eval(UTF8ToString($0));
                Module['sdl3_test_s'] = new TextEncoder().encode(String(v));
                return Module['sdl3_test_s'].length;
            "#,
                code.as_ptr()
            )
        };
        let mut buf = vec![0u8; len as usize];
        // SAFETY: writes the `len` bytes made above.
        unsafe {
            em_asm!(
                "HEAPU8.set(Module['sdl3_test_s'], $0); delete Module['sdl3_test_s'];",
                buf.as_mut_ptr()
            )
        };
        String::from_utf8(buf).unwrap()
    }

    /// Run JavaScript statements.
    fn js_run(code: &str) {
        js(&format!("(function() {{ {code} }})()"));
    }

    // -- the trace -------------------------------------------------------------

    static TRACE: Mutex<String> = Mutex::new(String::new());

    /// A line of the trace.
    macro_rules! out {
        ($($arg:tt)*) => {
            writeln!(TRACE.lock().unwrap(), $($arg)*).unwrap()
        };
    }

    /// Start a step's part of the trace.
    fn step(name: &str) {
        out!("== {name}");
    }

    fn sections(trace: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for line in trace.lines() {
            if let Some(name) = line.strip_prefix("== ") {
                out.push((name.to_owned(), String::new()));
            } else if let Some((_, body)) = out.last_mut() {
                body.push_str(line);
                body.push('\n');
            }
        }
        out
    }

    /// [`EXPECTED`] with this repository's two answers for the attributes
    /// (see the module's documentation).
    fn expected() -> String {
        EXPECTED
            .replace(
                "CONTEXT_RESET_NOTIFICATION=err(Unknown OpenGL attribute)",
                "CONTEXT_RESET_NOTIFICATION=0",
            )
            .replace(
                "FLOATBUFFERS=err()",
                "FLOATBUFFERS=err(That operation is not supported)",
            )
    }

    /// Compare the trace with upstream's, step by step; the result.
    fn finish() -> bool {
        let trace = TRACE.lock().unwrap().clone();
        let expected = sections(&expected());
        let actual = sections(&trace);
        if std::env::var_os("SDL3_TEST_TRACE").is_some() {
            println!("{trace}");
        }
        let mut passed = 0;
        let mut failed = Vec::new();
        for (name, want) in &expected {
            let got = actual.iter().find(|(n, _)| n == name).map(|(_, b)| b);
            if got == Some(want) {
                passed += 1;
                println!("test {name} ... ok");
            } else {
                println!("test {name} ... FAILED");
                println!(
                    "--- upstream's C:\n{want}--- here:\n{}",
                    got.map_or("(no such step)\n", |s| s)
                );
                failed.push(name.clone());
            }
        }
        for (name, body) in &actual {
            if !expected.iter().any(|(n, _)| n == name) {
                // (the steps of this program's own)
                let ok = !body.contains("FAILED");
                println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
                if ok {
                    passed += 1;
                } else {
                    println!("{body}");
                    failed.push(name.clone());
                }
            }
        }
        if !failed.is_empty() {
            println!("\nfailures:");
            for name in &failed {
                println!("    {name}");
            }
        }
        let status = if failed.is_empty() { "ok" } else { "FAILED" };
        println!(
            "\ntest result: {status}. {passed} passed; {} failed",
            failed.len()
        );
        failed.is_empty()
    }

    // -- GL --------------------------------------------------------------------

    type PGetError = unsafe extern "C" fn() -> u32;
    type PGetString = unsafe extern "C" fn(u32) -> *const u8;
    type PClearColor = unsafe extern "C" fn(f32, f32, f32, f32);
    type PClear = unsafe extern "C" fn(u32);
    type PReadPixels = unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void);
    type PGetIntegerv = unsafe extern "C" fn(u32, *mut c_int);

    /// The GL functions the steps call, from `SDL_GL_GetProcAddress()` (as
    /// the C's: function pointers into emscripten's WebGL library).
    #[derive(Clone, Copy)]
    struct Gl {
        get_error: PGetError,
        get_string: PGetString,
        clear_color: PClearColor,
        clear: PClear,
        read_pixels: PReadPixels,
        get_integerv: PGetIntegerv,
    }

    impl Gl {
        fn load() -> Gl {
            fn f<F: Copy>(name: &str) -> F {
                let p = gl::gl_get_proc_address(name).unwrap_or_else(|| panic!("{name}"));
                // SAFETY: F is the function pointer type of the entry point.
                unsafe { std::mem::transmute_copy::<*const c_void, F>(&p) }
            }
            Gl {
                get_error: f("glGetError"),
                get_string: f("glGetString"),
                clear_color: f("glClearColor"),
                clear: f("glClear"),
                read_pixels: f("glReadPixels"),
                get_integerv: f("glGetIntegerv"),
            }
        }

        fn string(&self, name: u32) -> String {
            // SAFETY: a context is current; NULL or GL's string.
            unsafe {
                let s = (self.get_string)(name);
                if s.is_null() {
                    "(null)".into()
                } else {
                    CStr::from_ptr(s as *const c_char)
                        .to_string_lossy()
                        .into_owned()
                }
            }
        }

        fn error(&self) -> u32 {
            // SAFETY: a plain GL call.
            unsafe { (self.get_error)() }
        }

        fn integer(&self, pname: u32) -> i32 {
            let mut v = -1;
            // SAFETY: one value written.
            unsafe { (self.get_integerv)(pname, &mut v) };
            v
        }

        /// The C's `clear_read()`: a clear, pixel (0, 0) read back, the
        /// error.
        fn clear_read(&self, label: &str, r: f32, g: f32, b: f32) {
            let mut px = [0xaau8; 4];
            // SAFETY: plain GL calls; 4 bytes read into `px`.
            unsafe {
                (self.clear_color)(r, g, b, 1.0);
                (self.clear)(0x4000);
                (self.read_pixels)(0, 0, 1, 1, 0x1908, 0x1401, px.as_mut_ptr().cast());
            }
            let err = self.error();
            out!(
                "{label} pixel {} {} {} {} error 0x{err:x}",
                px[0],
                px[1],
                px[2],
                px[3]
            );
        }
    }

    const VERSION_JS: &str = "(function() { var g = GL.currentContext.GLctx; return 'webgl' + GL.currentContext.version + ' ' + g.getParameter(g.VERSION) + ' / ' + g.getParameter(g.SHADING_LANGUAGE_VERSION); })()";

    fn canvas(id: &str) {
        js_run(&format!("var c = document.createElement('canvas'); c.id = '{id}'; c.width = 300; c.height = 150; document.body.appendChild(c);"));
    }

    fn window_on(title: &str, canvas_id: &str, w: i32, h: i32, flags: WindowFlags) -> Window {
        let p = Properties::new();
        p.set(PROP_WINDOW_CREATE_TITLE_STRING, title).unwrap();
        p.set(PROP_WINDOW_CREATE_WIDTH_NUMBER, w as i64).unwrap();
        p.set(PROP_WINDOW_CREATE_HEIGHT_NUMBER, h as i64).unwrap();
        p.set(PROP_WINDOW_CREATE_FLAGS_NUMBER, flags.0 as i64)
            .unwrap();
        p.set(PROP_WINDOW_CREATE_EMSCRIPTEN_CANVAS_ID_STRING, canvas_id)
            .unwrap();
        Window::create_with_properties(&p).unwrap()
    }

    const ATTRS: [(GlAttr, &str); 28] = [
        (GlAttr::RedSize, "RED_SIZE"),
        (GlAttr::GreenSize, "GREEN_SIZE"),
        (GlAttr::BlueSize, "BLUE_SIZE"),
        (GlAttr::AlphaSize, "ALPHA_SIZE"),
        (GlAttr::BufferSize, "BUFFER_SIZE"),
        (GlAttr::DoubleBuffer, "DOUBLEBUFFER"),
        (GlAttr::DepthSize, "DEPTH_SIZE"),
        (GlAttr::StencilSize, "STENCIL_SIZE"),
        (GlAttr::AccumRedSize, "ACCUM_RED_SIZE"),
        (GlAttr::AccumGreenSize, "ACCUM_GREEN_SIZE"),
        (GlAttr::AccumBlueSize, "ACCUM_BLUE_SIZE"),
        (GlAttr::AccumAlphaSize, "ACCUM_ALPHA_SIZE"),
        (GlAttr::Stereo, "STEREO"),
        (GlAttr::MultisampleBuffers, "MULTISAMPLEBUFFERS"),
        (GlAttr::MultisampleSamples, "MULTISAMPLESAMPLES"),
        (GlAttr::AcceleratedVisual, "ACCELERATED_VISUAL"),
        (GlAttr::RetainedBacking, "RETAINED_BACKING"),
        (GlAttr::ContextMajorVersion, "CONTEXT_MAJOR_VERSION"),
        (GlAttr::ContextMinorVersion, "CONTEXT_MINOR_VERSION"),
        (GlAttr::ContextFlags, "CONTEXT_FLAGS"),
        (GlAttr::ContextProfileMask, "CONTEXT_PROFILE_MASK"),
        (
            GlAttr::ShareWithCurrentContext,
            "SHARE_WITH_CURRENT_CONTEXT",
        ),
        (GlAttr::FramebufferSrgbCapable, "FRAMEBUFFER_SRGB_CAPABLE"),
        (GlAttr::ContextReleaseBehavior, "CONTEXT_RELEASE_BEHAVIOR"),
        (
            GlAttr::ContextResetNotification,
            "CONTEXT_RESET_NOTIFICATION",
        ),
        (GlAttr::ContextNoError, "CONTEXT_NO_ERROR"),
        (GlAttr::FloatBuffers, "FLOATBUFFERS"),
        (GlAttr::EglPlatform, "EGL_PLATFORM"),
    ];

    fn attributes(label: &str) {
        let mut line = format!("{label} attributes:");
        for (attr, name) in ATTRS {
            match gl::gl_get_attribute(attr) {
                Ok(v) => write!(line, " {name}={v}").unwrap(),
                Err(e) => write!(line, " {name}=err({})", e.message()).unwrap(),
            }
        }
        out!("{line}");
    }

    fn procs(label: &str) {
        let mut line = format!("{label} procs:");
        for name in [
            "glClear",
            "glDrawBuffers",
            "glBindVertexArray",
            "glGenVertexArraysOES",
            "glVertexAttribDivisorANGLE",
            "glNotAFunction",
            "glBegin",
        ] {
            write!(
                line,
                " {name}={}",
                gl::gl_get_proc_address(name).is_some() as i32
            )
            .unwrap();
        }
        out!("{line}");
    }

    /// The swap interval and emscripten's main loop timing.
    fn interval(label: &str) {
        let (mut mode, mut value): (c_int, c_int) = (-99, -99);
        // SAFETY: two ints to write.
        unsafe { sys::emscripten_get_main_loop_timing(&mut mode, &mut value) };
        match gl::gl_get_swap_interval() {
            Ok(i) => out!("{label} interval {i} timing {mode} {value}"),
            Err(_) => out!("{label} interval err 0 timing {mode} {value}"),
        }
    }

    fn set_interval(label: &str, i: i32) {
        match gl::gl_set_swap_interval(i) {
            Ok(()) => out!("{label} set {i}: ok"),
            Err(e) => out!("{label} set {i}: err {}", e.message()),
        }
        interval(label);
    }

    /// The C's `fnv()` and `pixel_report()`: the target's pixels as RGBA32,
    /// hashed, and six of them (at the offsets C computes, past a row's end
    /// for a narrow target too).
    fn pixel_report(r: &mut Renderer, label: &str) {
        let s = match r.read_pixels(None) {
            Ok(s) => s,
            Err(e) => {
                out!("scene {label}: read error {}", e.message());
                return;
            }
        };
        let c = s.convert(PixelFormat::RGBA32).unwrap();
        let (w, h, pitch) = (c.width(), c.height(), c.pitch() as usize);
        let px = c.pixels().unwrap();
        let mut hash: u32 = 2166136261;
        for y in 0..h as usize {
            for &b in &px[y * pitch..][..w as usize * 4] {
                hash ^= u32::from(b);
                hash = hash.wrapping_mul(16777619);
            }
        }
        let mut line = format!("scene {label}: {w}x{h} fnv {hash:08x}");
        for (x, y) in [(0, 0), (5, 6), (10, 10), (20, 12), (33, 20), (w - 1, h - 1)] {
            let p = &px[y as usize * pitch + x as usize * 4..][..4];
            write!(
                line,
                " ({x},{y})={:02x}{:02x}{:02x}{:02x}",
                p[0], p[1], p[2], p[3]
            )
            .unwrap();
        }
        out!("{line}");
    }

    fn fill_pattern(p: &mut [u8], pitch: usize, w: usize, h: usize) {
        for y in 0..h {
            for x in 0..w {
                let q = &mut p[y * pitch + x * 4..][..4];
                q[0] = (x * 16) as u8;
                q[1] = (y * 16) as u8;
                q[2] = ((x + y) * 8) as u8;
                q[3] = if (x / 4 + y / 4) % 2 != 0 { 255 } else { 128 };
            }
        }
    }

    fn static_texture(r: &mut Renderer, format: PixelFormat, w: i32, h: i32) -> Texture {
        r.create_texture(format, TextureAccess::Static, w, h)
            .unwrap()
    }

    /// The C's `scenes()`.
    fn scenes(r: &mut Renderer, tag: &str) {
        let label = |n: &str| format!("{tag} {n}");
        r.set_draw_blend_mode(BlendMode::NONE).unwrap();
        // clear
        r.set_draw_color(12, 34, 56, 255);
        r.clear().unwrap();
        pixel_report(r, &label("clear"));
        // shapes
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.set_draw_color(200, 100, 50, 255);
        r.render_fill_rect(Some(&FRect::new(4.0, 5.0, 20.0, 10.0)))
            .unwrap();
        r.set_draw_color(10, 220, 30, 255);
        r.render_fill_rects(&[
            FRect::new(30.0, 2.0, 7.0, 9.0),
            FRect::new(40.0, 30.0, 12.0, 3.0),
        ])
        .unwrap();
        r.set_draw_color(255, 255, 255, 255);
        r.render_points(&[
            FPoint { x: 1.0, y: 1.0 },
            FPoint { x: 62.0, y: 46.0 },
            FPoint { x: 33.0, y: 20.0 },
        ])
        .unwrap();
        r.set_draw_color(90, 80, 250, 255);
        r.render_line(2.0, 40.0, 60.0, 40.0).unwrap();
        r.render_line(50.0, 2.0, 50.0, 44.0).unwrap();
        r.render_rect(Some(&FRect::new(8.0, 24.0, 14.0, 10.0)))
            .unwrap();
        pixel_report(r, &label("shapes"));
        // blend
        r.set_draw_color(40, 80, 120, 200);
        r.clear().unwrap();
        for (i, mode) in [
            BlendMode::BLEND,
            BlendMode::ADD,
            BlendMode::MOD,
            BlendMode::MUL,
        ]
        .into_iter()
        .enumerate()
        {
            r.set_draw_blend_mode(mode).unwrap();
            r.set_draw_color(220, 60, 30, 128);
            r.render_fill_rect(Some(&FRect::new((i * 14) as f32, 6.0, 20.0, 30.0)))
                .unwrap();
        }
        r.set_draw_blend_mode(BlendMode::NONE).unwrap();
        pixel_report(r, &label("blend"));
        // texture
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        let t = static_texture(r, PixelFormat::RGBA32, 16, 16);
        let mut pat = vec![0u8; 16 * 16 * 4];
        fill_pattern(&mut pat, 64, 16, 16);
        r.update_texture(t, None, &pat, 64).unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.render_texture(t, None, Some(&FRect::new(2.0, 2.0, 32.0, 32.0)))
            .unwrap();
        r.set_texture_color_mod(t, 255, 128, 64).unwrap();
        r.set_texture_alpha_mod(t, 160).unwrap();
        r.render_texture_rotated(
            t,
            None,
            Some(&FRect::new(36.0, 4.0, 16.0, 16.0)),
            90.0,
            None,
            FlipMode::Horizontal,
        )
        .unwrap();
        r.destroy_texture(t);
        pixel_report(r, &label("texture"));
        // streaming
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        let t = r
            .create_texture(PixelFormat::ARGB8888, TextureAccess::Streaming, 8, 8)
            .unwrap();
        {
            let mut lock = r.lock_texture(t, None).unwrap();
            let pitch = lock.pitch() as usize;
            let pixels = lock.pixels();
            for y in 0..8 {
                for x in 0..8 {
                    let v: u32 = 0xff000000 | (x * 32) << 16 | (y * 32) << 8 | 0x40;
                    pixels[y as usize * pitch + x as usize * 4..][..4]
                        .copy_from_slice(&v.to_ne_bytes());
                }
            }
        }
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.render_texture(t, None, Some(&FRect::new(8.0, 8.0, 24.0, 24.0)))
            .unwrap();
        r.destroy_texture(t);
        pixel_report(r, &label("streaming"));
        // yuv
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        let yp: Vec<u8> = (0..256).map(|i| (16 + (i % 16) * 13) as u8).collect();
        let up: Vec<u8> = (0..64).map(|i| (64 + i * 2) as u8).collect();
        let vp: Vec<u8> = (0..64).map(|i| (200 - i * 2) as u8).collect();
        let uv: Vec<u8> = up.iter().zip(&vp).flat_map(|(u, v)| [*u, *v]).collect();
        let t = static_texture(r, PixelFormat::IYUV, 16, 16);
        r.update_yuv_texture(t, None, &yp, 16, &up, 8, &vp, 8)
            .unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.render_texture(t, None, Some(&FRect::new(0.0, 0.0, 32.0, 32.0)))
            .unwrap();
        r.destroy_texture(t);
        let t = static_texture(r, PixelFormat::NV12, 16, 16);
        r.update_nv_texture(t, None, &yp, 16, &uv, 16).unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.render_texture(t, None, Some(&FRect::new(32.0, 0.0, 32.0, 32.0)))
            .unwrap();
        r.destroy_texture(t);
        pixel_report(r, &label("yuv"));
        // target
        let tt = r
            .create_texture(PixelFormat::RGBA32, TextureAccess::Target, 32, 24)
            .unwrap();
        r.set_render_target(Some(tt)).unwrap();
        r.set_draw_color(10, 20, 30, 255);
        r.clear().unwrap();
        r.set_draw_color(250, 200, 0, 255);
        r.render_fill_rect(Some(&FRect::new(4.0, 4.0, 10.0, 6.0)))
            .unwrap();
        pixel_report(r, &label("target itself"));
        r.set_render_target(None).unwrap();
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.set_texture_scale_mode(tt, ScaleMode::Nearest).unwrap();
        r.render_texture(tt, None, Some(&FRect::new(16.0, 12.0, 32.0, 24.0)))
            .unwrap();
        r.destroy_texture(tt);
        pixel_report(r, &label("target"));
        // geometry
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        let gc = FColor::new(1.0, 0.5, 0.25, 1.0);
        let corners = [(8.0, 8.0), (40.0, 8.0), (40.0, 40.0), (8.0, 40.0)];
        let tc = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let mut v: Vec<Vertex> = corners
            .iter()
            .zip(tc)
            .map(|(&(x, y), (u, w))| Vertex {
                position: FPoint { x, y },
                color: gc,
                tex_coord: FPoint { x: u, y: w },
            })
            .collect();
        let idx = [0, 1, 2, 0, 2, 3];
        r.render_geometry(None, &v, Some(&idx)).unwrap();
        let t = static_texture(r, PixelFormat::RGBA32, 16, 16);
        r.update_texture(t, None, &pat, 64).unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        for v in &mut v {
            v.position.x += 20.0;
            v.position.y += 4.0;
            v.color = FColor::new(1.0, 1.0, 1.0, 1.0);
        }
        r.render_geometry(Some(t), &v, Some(&idx)).unwrap();
        r.destroy_texture(t);
        pixel_report(r, &label("geometry"));
    }

    // -- the synchronous steps ---------------------------------------------------

    /// What the steps keep for the main loop (on the main thread: the
    /// main callbacks' state must be `Send`, a renderer isn't).
    struct State {
        gl: Option<Gl>,
        w1: Option<Window>,
        c1: Option<GlContext>,
        rw: Option<Window>,
        ren: Option<Renderer>,
    }

    fn init_step() {
        step("init");
        init::init(InitFlags::VIDEO).unwrap();
        out!("driver {}", sdl_video::current_video_driver().unwrap());
        attributes("defaults");
        for id in ["gl1", "gl2", "gl3", "gl4"] {
            canvas(id);
        }
    }

    fn es2_step(st: &mut State) {
        step("es2");
        let w1 = window_on("es2", "#gl1", 64, 48, WindowFlags::OPENGL);
        out!("es2 window flags 0x{:x}", w1.flags().unwrap().0);
        let c1 = GlContext::new(&w1).unwrap();
        out!(
            "es2 context 1 current {} window {}",
            (gl::gl_current_context().unwrap() == Some(c1.raw())) as i32,
            (gl::gl_current_window().unwrap().map(|w| w.id()) == Some(w1.id())) as i32
        );
        let g = Gl::load();
        st.gl = Some(g);
        out!(
            "es2 version [{}] glsl [{}] vendor [{}] context [{}]",
            g.string(0x1F02),
            g.string(0x8B8C),
            g.string(0x1F00),
            js(VERSION_JS)
        );
        let m = g.integer(0x821B);
        out!("es2 major {m} error 0x{:x}", g.error());
        attributes("es2");
        procs("es2");
        g.clear_read("es2", 0.25, 0.5, 0.75);
        match GlContext::new(&w1) {
            Ok(_) => out!("es2 second context 1"),
            Err(e) => out!("es2 second context 0: {}", e.message()),
        }
        out!("es2 swap {}", gl::gl_swap_window(&w1).is_ok() as i32);
        set_interval("before", -1);
        set_interval("before", 1);
        set_interval("before", 0);
        let w2 = window_on("other", "#gl2", 32, 32, WindowFlags::OPENGL);
        match gl::gl_make_current(Some(&w2), Some(&c1)) {
            Ok(()) => out!("other window make current 1"),
            Err(e) => out!("other window make current 0: {}", e.message()),
        }
        out!(
            "release {} current {}",
            gl::gl_release_current().is_ok() as i32,
            gl::gl_current_context().unwrap().is_some() as i32
        );
        match gl::gl_swap_window(&w2) {
            Ok(()) => out!("other window swap 1"),
            Err(e) => out!("other window swap 0: {}", e.message()),
        }
        out!(
            "make current {} current {}",
            c1.make_current(Some(&w1)).is_ok() as i32,
            c1.is_current() as i32
        );
        c1.destroy().unwrap();
        out!(
            "destroyed current {}",
            gl::gl_current_context().unwrap().is_some() as i32
        );
        let c1 = GlContext::new(&w1);
        out!("es2 again {}", c1.is_ok() as i32);
        st.c1 = c1.ok();
        st.w1 = Some(w1);
        w2.destroy();
    }

    fn es3_step(st: &State) {
        step("es3");
        let g = st.gl.unwrap();
        gl::gl_set_attribute(GlAttr::ContextMajorVersion, 3).unwrap();
        gl::gl_set_attribute(GlAttr::ContextMinorVersion, 0).unwrap();
        let w3 = window_on("es3", "#gl3", 64, 48, WindowFlags::OPENGL);
        let c3 = GlContext::new(&w3);
        out!("es3 context {}", c3.is_ok() as i32);
        let major = g.integer(0x821B);
        out!(
            "es3 version [{}] glsl [{}] (emscripten's cache) major {major} error 0x{:x} context [{}]",
            g.string(0x1F02),
            g.string(0x8B8C),
            g.error(),
            js(VERSION_JS)
        );
        attributes("es3");
        procs("es3");
        g.clear_read("es3", 0.75, 0.5, 0.25);
        drop(c3);
        w3.destroy();
    }

    fn attrs_step() {
        step("attrs");
        gl::gl_reset_attributes();
        gl::gl_set_attribute(GlAttr::AlphaSize, 0).unwrap();
        gl::gl_set_attribute(GlAttr::DepthSize, 0).unwrap();
        gl::gl_set_attribute(GlAttr::StencilSize, 8).unwrap();
        gl::gl_set_attribute(GlAttr::MultisampleBuffers, 1).unwrap();
        gl::gl_set_attribute(GlAttr::MultisampleSamples, 4).unwrap();
        let w4 = window_on("attrs", "#gl4", 64, 48, WindowFlags::OPENGL);
        let c4 = GlContext::new(&w4);
        out!(
            "attrs context {}: {}",
            c4.is_ok() as i32,
            js("JSON.stringify(GL.currentContext.GLctx.getContextAttributes())")
        );
        attributes("attrs");
        drop(c4);
        w4.destroy();
        gl::gl_reset_attributes();
    }

    fn renderer_step(st: &mut State) {
        step("renderer");
        let rw = Window::create("render", 64, 48, WindowFlags::NONE).unwrap();
        let mut ren = Renderer::for_window(&rw, Some("opengles2")).unwrap();
        out!(
            "renderer {} flags 0x{:x} max texture {}",
            ren.name(),
            rw.flags().unwrap().0,
            ren.properties()
                .get_number(PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER)
                .unwrap_or(0)
        );
        let formats: Vec<&str> = ren.texture_formats().iter().map(|f| f.name()).collect();
        out!("formats: {}", formats.join(" "));
        attributes("renderer");
        scenes(&mut ren, "es2");
        let set = ren.set_vsync(1).is_ok() as i32;
        out!("vsync set {set} get {}", ren.vsync());
        interval("vsync");
        out!("present {}", ren.present().is_ok() as i32);
        st.rw = Some(rw);
        st.ren = Some(ren);
    }

    fn renderer_es3_step() {
        step("renderer_es3");
        gl::gl_set_attribute(GlAttr::ContextMajorVersion, 3).unwrap();
        gl::gl_set_attribute(GlAttr::ContextMinorVersion, 0).unwrap();
        gl::gl_set_attribute(GlAttr::ContextProfileMask, gl::GL_CONTEXT_PROFILE_ES).unwrap();
        let w5 = window_on("render3", "#gl3", 64, 48, WindowFlags::OPENGL);
        let mut r3 = Renderer::for_window(&w5, Some("opengles2")).unwrap();
        out!("es3 renderer {} context [{}]", r3.name(), js(VERSION_JS));
        scenes(&mut r3, "es3");
        // (this program's own step: the renderer's last program is made of
        // its OPENGLES_300 shaders, GLSL ES 3.00, which upstream's C
        // doesn't have)
        step("renderer_es3_shaders");
        let versions = js("(function() { var g = GL.currentContext.GLctx; var p = g.getParameter(g.CURRENT_PROGRAM); return g.getAttachedShaders(p).map((s) => g.getShaderSource(s).split(String.fromCharCode(10))[0]).join(' | '); })()");
        if versions.matches("#version 300 es").count() == 2 {
            out!("{versions}");
        } else {
            out!("FAILED: not the OPENGLES_300 shaders: {versions}");
        }
        drop(r3);
        w5.destroy();
        gl::gl_reset_attributes();
    }

    /// `SDL_GL_ExtensionSupported()` against the context's list (this
    /// program's own step: the C trace hasn't it).
    fn extensions_step(st: &State) {
        step("extensions");
        let (Some(w1), Some(c1), Some(g)) = (st.w1, st.c1.as_ref(), st.gl) else {
            out!("FAILED: no context");
            return;
        };
        c1.make_current(Some(&w1)).unwrap();
        // (the first context's list: emscripten's glGetString() keeps it)
        let list = g.string(0x1F03);
        let mut bad = Vec::new();
        for ext in list.split(' ').filter(|e| !e.is_empty()) {
            if !gl::gl_extension_supported(ext) {
                bad.push(ext.to_owned());
            }
        }
        for ext in [
            "GL_NOT_AN_EXTENSION",
            "GL_",
            "",
            "GL_OES_texture_float GL_OES",
        ] {
            if gl::gl_extension_supported(ext) {
                bad.push(format!("{ext:?} (supported)"));
            }
        }
        if let Some(first) = list.split(' ').find(|e| e.starts_with("GL_")) {
            hints::set(first, "0").unwrap();
            if gl::gl_extension_supported(first) {
                bad.push(format!("{first} (hint 0)"));
            }
            hints::reset(first);
        }
        let n = list.split(' ').filter(|e| !e.is_empty()).count();
        if n == 0 || !bad.is_empty() {
            out!("FAILED: {n} extensions, wrong: {bad:?}");
        } else {
            out!("{n} extensions");
        }
    }

    pub(super) fn main() -> ExitCode {
        let skip = |capability: &str, reason: &str| {
            if required(capability) {
                panic!("required capability {capability} unavailable: {reason}");
            }
            println!("note: skipping, {capability} unavailable: {reason}");
            let steps = sections(EXPECTED);
            for (name, _) in &steps {
                println!("test {name} ... ok");
            }
            for name in OWN_STEPS {
                println!("test {name} ... ok");
            }
            println!(
                "\ntest result: ok. {} passed; 0 failed",
                steps.len() + OWN_STEPS.len()
            );
            ExitCode::SUCCESS
        };
        if !in_browser() {
            return skip(
                "browser",
                "not in a browser (under node: run it with SDL3_EMSCRIPTEN_BROWSER=1, or with CHROME naming Chrome or Chromium)",
            );
        }
        if !has_webgl() {
            return skip(
                "webgl",
                "no WebGL context (Chromium needs --enable-unsafe-swiftshader --use-angle=swiftshader without a GPU)",
            );
        }
        ExitCode::from(app::run_app(sdl_main) as u8)
    }

    /// The steps of this program's own, which the C trace hasn't.
    const OWN_STEPS: [&str; 2] = ["renderer_es3_shaders", "extensions"];

    /// Run a synchronous step; a panic is recorded in its trace.
    fn guarded(name: &str, f: impl FnOnce()) {
        if panic::catch_unwind(AssertUnwindSafe(f)).is_err() {
            out!("FAILED: {name} panicked");
        }
    }

    thread_local! {
        static STATE: std::cell::RefCell<State> = const {
            std::cell::RefCell::new(State {
                gl: None,
                w1: None,
                c1: None,
                rw: None,
                ren: None,
            })
        };
    }

    fn sdl_main(args: &[String]) -> i32 {
        STATE.with_borrow_mut(synchronous_steps);
        app::enter_app_main_callbacks(args, move |_| {
            (App { step: 0, frames: 0 }, AppResult::Continue)
        })
    }

    fn synchronous_steps(st: &mut State) {
        guarded("init", init_step);
        guarded("es2", || es2_step(st));
        guarded("es3", || es3_step(st));
        guarded("attrs", attrs_step);
        guarded("renderer", || renderer_step(st));
        guarded("renderer_es3", renderer_es3_step);
        guarded("extensions", || extensions_step(st));
        guarded("enter", || {
            step("enter");
            if let (Some(w1), Some(c1)) = (st.w1, st.c1.as_ref()) {
                c1.make_current(Some(&w1)).unwrap();
            }
            set_interval("enter", 2);
        });
    }

    // -- the main callbacks ----------------------------------------------------

    struct App {
        step: u32,
        frames: u32,
    }

    impl App {
        fn iterate_step(&mut self, st: &mut State) -> Option<AppResult> {
            self.frames += 1;
            match self.step {
                0 => {
                    step("iter1");
                    interval("iter1");
                    set_interval("iter1", 0);
                    set_interval("iter1", 1);
                    hints::set(hints::MAIN_CALLBACK_RATE, "30").unwrap();
                    self.step = 1;
                }
                1 => {
                    step("iter2");
                    interval("iter2 (rate 30)");
                    set_interval("iter2 (rate 30)", 1);
                    hints::reset(hints::MAIN_CALLBACK_RATE);
                    self.step = 2;
                }
                2 => {
                    step("iter3");
                    interval("iter3 (rate reset)");
                    set_interval("iter3", 1);
                    // the renderer's present shows on the canvas
                    let ren = st.ren.as_mut().unwrap();
                    ren.set_draw_color(30, 60, 90, 255);
                    ren.clear().unwrap();
                    out!("iter3 present {}", ren.present().is_ok() as i32);
                    out!("iter3 canvas {}", js("(function() { var c = document.getElementById('canvas'); var o = document.createElement('canvas'); o.width = c.width; o.height = c.height; var g = o.getContext('2d'); g.drawImage(c, 0, 0); return c.width + 'x' + c.height + ' ' + Array.from(g.getImageData(1, 1, 1, 1).data).join(','); })()"));
                    // context loss
                    step("loss");
                    let (w1, c1, g) = (st.w1.unwrap(), st.c1.as_ref().unwrap(), st.gl.unwrap());
                    let _ = c1.make_current(Some(&w1));
                    js_run("Module.sdl3Restored = 0; var gl = GL.currentContext.GLctx; gl.canvas.addEventListener('webglcontextlost', (e) => { Module.sdl3Lost = (Module.sdl3Lost || 0) + 1; if (Module.sdl3Restore) e.preventDefault(); }); gl.canvas.addEventListener('webglcontextrestored', () => { Module.sdl3Restored++; }); Module.sdl3Restore = 1; Module.sdl3LoseExt = gl.getExtension('WEBGL_lose_context'); Module.sdl3LoseExt.loseContext();");
                    out!(
                        "lost: {} events {}",
                        js("String(GL.currentContext.GLctx.isContextLost())"),
                        js("String(Module.sdl3Lost)")
                    );
                    g.clear_read("lost", 1.0, 0.0, 0.0);
                    // (the current context: the front end's no-op)
                    let swap = gl::gl_swap_window(&w1).is_ok() as i32;
                    let current = gl::gl_make_current(Some(&w1), Some(c1)).is_ok() as i32;
                    out!("lost swap {swap} make current {current}");
                    self.step = 3;
                    self.frames = 0;
                }
                3 => {
                    // (the browser's webglcontextlost comes from its event
                    // loop; once it's prevented, the context can be restored)
                    if js("String(Module.sdl3Lost || 0)").parse::<i32>().unwrap() > 0 {
                        let w1 = st.w1.unwrap();
                        out!(
                            "lost event after {} frames, swap {}",
                            (self.frames > 0) as i32,
                            gl::gl_swap_window(&w1).is_ok() as i32
                        );
                        js_run("Module.sdl3LoseExt.restoreContext();");
                        self.step = 5;
                        self.frames = 0;
                    } else if self.frames > 300 {
                        out!("no lost event");
                        self.step = 4;
                    }
                }
                5 => {
                    if js("String(Module.sdl3Restored)").parse::<i32>().unwrap() > 0 {
                        let (w1, c1, g) = (st.w1.unwrap(), st.c1.as_ref().unwrap(), st.gl.unwrap());
                        let _ = c1.make_current(Some(&w1));
                        out!(
                            "restored: {}",
                            js("String(GL.currentContext.GLctx.isContextLost())")
                        );
                        g.clear_read("restored", 0.0, 1.0, 0.0);
                        self.step = 4;
                    } else if self.frames > 300 {
                        out!("not restored");
                        self.step = 4;
                    }
                }
                _ => {
                    step("quit");
                    drop(st.ren.take());
                    drop(st.c1.take());
                    if let Some(w) = st.w1.take() {
                        w.destroy();
                    }
                    if let Some(w) = st.rw.take() {
                        w.destroy();
                    }
                    init::quit();
                    return Some(if finish() {
                        AppResult::Success
                    } else {
                        AppResult::Failure
                    });
                }
            }
            None
        }
    }

    impl AppCallbacks for App {
        fn iterate(&mut self) -> AppResult {
            match panic::catch_unwind(AssertUnwindSafe(|| {
                STATE.with_borrow_mut(|st| self.iterate_step(st))
            })) {
                Ok(Some(result)) => result,
                Ok(None) => AppResult::Continue,
                Err(_) => {
                    out!("FAILED: step {} panicked", self.step);
                    if self.step == 4 {
                        // (in the end itself)
                        return AppResult::Failure;
                    }
                    self.step = 4;
                    AppResult::Continue
                }
            }
        }

        fn event(&mut self, _event: &Event) -> AppResult {
            AppResult::Continue
        }

        fn quit(&mut self, _result: AppResult) {}
    }

    /// The trace of upstream's C on the same page with the same steps.
    const EXPECTED: &str = r#"== init
driver emscripten
defaults attributes: RED_SIZE=err(No GL driver has been loaded) GREEN_SIZE=err(No GL driver has been loaded) BLUE_SIZE=err(No GL driver has been loaded) ALPHA_SIZE=err(No GL driver has been loaded) BUFFER_SIZE=err(No GL driver has been loaded) DOUBLEBUFFER=1 DEPTH_SIZE=err(No GL driver has been loaded) STENCIL_SIZE=err(No GL driver has been loaded) ACCUM_RED_SIZE=0 ACCUM_GREEN_SIZE=0 ACCUM_BLUE_SIZE=0 ACCUM_ALPHA_SIZE=0 STEREO=0 MULTISAMPLEBUFFERS=err(No GL driver has been loaded) MULTISAMPLESAMPLES=err(No GL driver has been loaded) ACCELERATED_VISUAL=1 RETAINED_BACKING=1 CONTEXT_MAJOR_VERSION=2 CONTEXT_MINOR_VERSION=0 CONTEXT_FLAGS=0 CONTEXT_PROFILE_MASK=4 SHARE_WITH_CURRENT_CONTEXT=0 FRAMEBUFFER_SRGB_CAPABLE=0 CONTEXT_RELEASE_BEHAVIOR=err(No GL driver has been loaded) CONTEXT_RESET_NOTIFICATION=err(Unknown OpenGL attribute) CONTEXT_NO_ERROR=0 FLOATBUFFERS=err() EGL_PLATFORM=0
== es2
es2 window flags 0x602
es2 context 1 current 1 window 1
es2 version [OpenGL ES 2.0 (WebGL 1.0 (OpenGL ES 2.0 Chromium))] glsl [OpenGL ES GLSL ES 1.00 (WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0 Chromium))] vendor [WebKit] context [webgl1 WebGL 1.0 (OpenGL ES 2.0 Chromium) / WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0 Chromium)]
es2 major -1 error 0x500
es2 attributes: RED_SIZE=8 GREEN_SIZE=8 BLUE_SIZE=8 ALPHA_SIZE=8 BUFFER_SIZE=32 DOUBLEBUFFER=1 DEPTH_SIZE=24 STENCIL_SIZE=0 ACCUM_RED_SIZE=0 ACCUM_GREEN_SIZE=0 ACCUM_BLUE_SIZE=0 ACCUM_ALPHA_SIZE=0 STEREO=0 MULTISAMPLEBUFFERS=0 MULTISAMPLESAMPLES=0 ACCELERATED_VISUAL=1 RETAINED_BACKING=1 CONTEXT_MAJOR_VERSION=2 CONTEXT_MINOR_VERSION=0 CONTEXT_FLAGS=0 CONTEXT_PROFILE_MASK=4 SHARE_WITH_CURRENT_CONTEXT=0 FRAMEBUFFER_SRGB_CAPABLE=0 CONTEXT_RELEASE_BEHAVIOR=err(OpenGL error: GL_INVALID_ENUM) CONTEXT_RESET_NOTIFICATION=err(Unknown OpenGL attribute) CONTEXT_NO_ERROR=0 FLOATBUFFERS=err() EGL_PLATFORM=0
es2 procs: glClear=1 glDrawBuffers=1 glBindVertexArray=1 glGenVertexArraysOES=1 glVertexAttribDivisorANGLE=1 glNotAFunction=0 glBegin=0
es2 pixel 64 128 191 255 error 0x0
es2 second context 0: Cannot create multiple webgl contexts per window
es2 swap 1
before set -1: err Late swap tearing currently unsupported
before interval 0 timing 0 0
before set 1: ok
before interval 1 timing 0 0
before set 0: ok
before interval 0 timing 0 0
other window make current 0: Cannot make context current to another window
release 1 current 0
other window swap 0: The specified window has not been made current
make current 1 current 1
destroyed current 0
es2 again 1
== es3
es3 context 1
es3 version [OpenGL ES 2.0 (WebGL 1.0 (OpenGL ES 2.0 Chromium))] glsl [OpenGL ES GLSL ES 1.00 (WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0 Chromium))] (emscripten's cache) major 3 error 0x0 context [webgl2 WebGL 2.0 (OpenGL ES 3.0 Chromium) / WebGL GLSL ES 3.00 (OpenGL ES GLSL ES 3.0 Chromium)]
es3 attributes: RED_SIZE=8 GREEN_SIZE=8 BLUE_SIZE=8 ALPHA_SIZE=8 BUFFER_SIZE=32 DOUBLEBUFFER=1 DEPTH_SIZE=24 STENCIL_SIZE=0 ACCUM_RED_SIZE=0 ACCUM_GREEN_SIZE=0 ACCUM_BLUE_SIZE=0 ACCUM_ALPHA_SIZE=0 STEREO=0 MULTISAMPLEBUFFERS=0 MULTISAMPLESAMPLES=0 ACCELERATED_VISUAL=1 RETAINED_BACKING=1 CONTEXT_MAJOR_VERSION=3 CONTEXT_MINOR_VERSION=0 CONTEXT_FLAGS=0 CONTEXT_PROFILE_MASK=4 SHARE_WITH_CURRENT_CONTEXT=0 FRAMEBUFFER_SRGB_CAPABLE=0 CONTEXT_RELEASE_BEHAVIOR=err(OpenGL error: GL_INVALID_ENUM) CONTEXT_RESET_NOTIFICATION=err(Unknown OpenGL attribute) CONTEXT_NO_ERROR=0 FLOATBUFFERS=err() EGL_PLATFORM=0
es3 procs: glClear=1 glDrawBuffers=1 glBindVertexArray=1 glGenVertexArraysOES=1 glVertexAttribDivisorANGLE=1 glNotAFunction=0 glBegin=0
es3 pixel 191 128 64 255 error 0x0
== attrs
attrs context 1: {"alpha":false,"antialias":true,"depth":false,"desynchronized":false,"failIfMajorPerformanceCaveat":false,"powerPreference":"default","premultipliedAlpha":true,"preserveDrawingBuffer":false,"stencil":true,"xrCompatible":false}
attrs attributes: RED_SIZE=8 GREEN_SIZE=8 BLUE_SIZE=8 ALPHA_SIZE=0 BUFFER_SIZE=24 DOUBLEBUFFER=1 DEPTH_SIZE=0 STENCIL_SIZE=8 ACCUM_RED_SIZE=0 ACCUM_GREEN_SIZE=0 ACCUM_BLUE_SIZE=0 ACCUM_ALPHA_SIZE=0 STEREO=0 MULTISAMPLEBUFFERS=1 MULTISAMPLESAMPLES=4 ACCELERATED_VISUAL=1 RETAINED_BACKING=1 CONTEXT_MAJOR_VERSION=2 CONTEXT_MINOR_VERSION=0 CONTEXT_FLAGS=0 CONTEXT_PROFILE_MASK=4 SHARE_WITH_CURRENT_CONTEXT=0 FRAMEBUFFER_SRGB_CAPABLE=0 CONTEXT_RELEASE_BEHAVIOR=err(OpenGL error: GL_INVALID_ENUM) CONTEXT_RESET_NOTIFICATION=err(Unknown OpenGL attribute) CONTEXT_NO_ERROR=0 FLOATBUFFERS=err() EGL_PLATFORM=0
== renderer
renderer opengles2 flags 0x602 max texture 8192
formats: SDL_PIXELFORMAT_ARGB8888 SDL_PIXELFORMAT_ABGR8888 SDL_PIXELFORMAT_XRGB8888 SDL_PIXELFORMAT_XBGR8888 SDL_PIXELFORMAT_INDEX8 SDL_PIXELFORMAT_YV12 SDL_PIXELFORMAT_IYUV SDL_PIXELFORMAT_I444 SDL_PIXELFORMAT_NV12 SDL_PIXELFORMAT_NV21
renderer attributes: RED_SIZE=8 GREEN_SIZE=8 BLUE_SIZE=8 ALPHA_SIZE=8 BUFFER_SIZE=32 DOUBLEBUFFER=1 DEPTH_SIZE=24 STENCIL_SIZE=0 ACCUM_RED_SIZE=0 ACCUM_GREEN_SIZE=0 ACCUM_BLUE_SIZE=0 ACCUM_ALPHA_SIZE=0 STEREO=0 MULTISAMPLEBUFFERS=0 MULTISAMPLESAMPLES=0 ACCELERATED_VISUAL=1 RETAINED_BACKING=1 CONTEXT_MAJOR_VERSION=2 CONTEXT_MINOR_VERSION=0 CONTEXT_FLAGS=0 CONTEXT_PROFILE_MASK=4 SHARE_WITH_CURRENT_CONTEXT=0 FRAMEBUFFER_SRGB_CAPABLE=0 CONTEXT_RELEASE_BEHAVIOR=err(OpenGL error: GL_INVALID_ENUM) CONTEXT_RESET_NOTIFICATION=err(Unknown OpenGL attribute) CONTEXT_NO_ERROR=0 FLOATBUFFERS=err() EGL_PLATFORM=0
scene es2 clear: 64x48 fnv 53458dc5 (0,0)=0c2238ff (5,6)=0c2238ff (10,10)=0c2238ff (20,12)=0c2238ff (33,20)=0c2238ff (63,47)=0c2238ff
scene es2 shapes: 64x48 fnv a93aac6c (0,0)=000000ff (5,6)=c86432ff (10,10)=c86432ff (20,12)=c86432ff (33,20)=ffffffff (63,47)=000000ff
scene es2 blend: 64x48 fnv d350392d (0,0)=285078c8 (5,6)=82464be4 (10,10)=82464be4 (20,12)=966e87c8 (33,20)=811a10c8 (63,47)=285078c8
scene es2 texture: 64x48 fnv 559d97b5 (0,0)=000000ff (5,6)=08100cff (10,10)=202020ff (20,12)=905070ff (33,20)=f090c0ff (63,47)=000000ff
scene es2 streaming: 64x48 fnv 9a3d8fc5 (0,0)=000000ff (5,6)=000000ff (10,10)=000040ff (20,12)=802040ff (33,20)=000000ff (63,47)=000000ff
scene es2 yuv: 64x48 fnv 316f6d45 (0,0)=730000ff (5,6)=740400ff (10,10)=853913ff (20,12)=ae8e8bff (33,20)=000020ff (63,47)=000000ff
scene es2 target itself: 32x24 fnv 608c71fd (0,0)=0a141eff (5,6)=fac800ff (10,10)=0a141eff (20,12)=0a141eff (33,20)=0a141eff (31,23)=0a141eff
scene es2 target: 64x48 fnv 543b2dfd (0,0)=000000ff (5,6)=000000ff (10,10)=000000ff (20,12)=0a141eff (33,20)=0a141eff (63,47)=000000ff
scene es2 geometry: 64x48 fnv 05453fa5 (0,0)=000000ff (5,6)=000000ff (10,10)=ff8040ff (20,12)=ff8040ff (33,20)=204030ff (63,47)=000000ff
vsync set 1 get 1
vsync interval 1 timing 0 0
present 1
== renderer_es3
es3 renderer opengles2 context [webgl2 WebGL 2.0 (OpenGL ES 3.0 Chromium) / WebGL GLSL ES 3.00 (OpenGL ES GLSL ES 3.0 Chromium)]
scene es3 clear: 64x48 fnv 53458dc5 (0,0)=0c2238ff (5,6)=0c2238ff (10,10)=0c2238ff (20,12)=0c2238ff (33,20)=0c2238ff (63,47)=0c2238ff
scene es3 shapes: 64x48 fnv a93aac6c (0,0)=000000ff (5,6)=c86432ff (10,10)=c86432ff (20,12)=c86432ff (33,20)=ffffffff (63,47)=000000ff
scene es3 blend: 64x48 fnv d350392d (0,0)=285078c8 (5,6)=82464be4 (10,10)=82464be4 (20,12)=966e87c8 (33,20)=811a10c8 (63,47)=285078c8
scene es3 texture: 64x48 fnv 559d97b5 (0,0)=000000ff (5,6)=08100cff (10,10)=202020ff (20,12)=905070ff (33,20)=f090c0ff (63,47)=000000ff
scene es3 streaming: 64x48 fnv 9a3d8fc5 (0,0)=000000ff (5,6)=000000ff (10,10)=000040ff (20,12)=802040ff (33,20)=000000ff (63,47)=000000ff
scene es3 yuv: 64x48 fnv 316f6d45 (0,0)=730000ff (5,6)=740400ff (10,10)=853913ff (20,12)=ae8e8bff (33,20)=000020ff (63,47)=000000ff
scene es3 target itself: 32x24 fnv 608c71fd (0,0)=0a141eff (5,6)=fac800ff (10,10)=0a141eff (20,12)=0a141eff (33,20)=0a141eff (31,23)=0a141eff
scene es3 target: 64x48 fnv 543b2dfd (0,0)=000000ff (5,6)=000000ff (10,10)=000000ff (20,12)=0a141eff (33,20)=0a141eff (63,47)=000000ff
scene es3 geometry: 64x48 fnv 05453fa5 (0,0)=000000ff (5,6)=000000ff (10,10)=ff8040ff (20,12)=ff8040ff (33,20)=204030ff (63,47)=000000ff
== enter
enter set 2: ok
enter interval 2 timing 0 0
== iter1
iter1 interval 2 timing 1 2
iter1 set 0: ok
iter1 interval 0 timing 0 0
iter1 set 1: ok
iter1 interval 1 timing 1 1
== iter2
iter2 (rate 30) interval 0 timing 0 33
iter2 (rate 30) set 1: ok
iter2 (rate 30) interval 0 timing 0 33
== iter3
iter3 (rate reset) interval 1 timing 1 1
iter3 set 1: ok
iter3 interval 1 timing 1 1
iter3 present 1
iter3 canvas 64x48 30,60,90,255
== loss
lost: true events undefined
lost pixel 170 170 170 170 error 0x9242
lost swap 1 make current 1
lost event after 1 frames, swap 1
restored: false
restored pixel 0 255 0 255 error 0x0
== quit
"#;
}
