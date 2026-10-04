// Tests of the OpenGL renderer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The scenes are drawn by the OpenGL renderer on a real context and by the
//! software renderer, and the pixels compared: exactly where the math is
//! exact (fills, points, 1:1 copies, palettes), within a small tolerance
//! where GL's float blending or YUV conversion rounds differently.
//!
//! The contexts come from the offscreen driver's EGL (Mesa's device
//! platform) and, when `DISPLAY` is set (`xvfb-run`), X11's GLX; on Windows
//! from WGL. Without a usable OpenGL (no library, or one without
//! framebuffer objects such as GDI Generic 1.1) the tests print a note and
//! pass.

use super::shaders::{self, Shader};
use super::*;
use crate::events::window::WindowFlags;
use crate::hints;
use crate::init::{self, InitFlags};
use crate::render::{
    Renderer, TextureCreateInfo, Vertex, PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER,
    PROP_RENDERER_TEXTURE_WRAPPING_BOOLEAN,
};
use crate::video::blendmode::{BlendFactor as BF, BlendOperation as BO};
use crate::video::pixels::Palette;
use crate::video::surface::share_palette;

const W: i32 = 64;
const H: i32 = 48;

// ---------------------------------------------------------------------------
// Without a context
// ---------------------------------------------------------------------------

#[test]
fn powerof2_like_upstream() {
    assert_eq!(powerof2(-3), 1);
    assert_eq!(powerof2(0), 1);
    assert_eq!(powerof2(1), 1);
    assert_eq!(powerof2(2), 2);
    assert_eq!(powerof2(3), 4);
    assert_eq!(powerof2(640), 1024);
    assert_eq!(powerof2(1024), 1024);
    assert_eq!(powerof2(1025), 2048);
}

#[test]
fn error_names() {
    assert_eq!(translate_error(0x0500), "GL_INVALID_ENUM");
    assert_eq!(translate_error(0x0502), "GL_INVALID_OPERATION");
    assert_eq!(translate_error(0x0505), "GL_OUT_OF_MEMORY");
    assert_eq!(translate_error(0x8031), "GL_TABLE_TOO_LARGE");
    assert_eq!(translate_error(0), "GL_NO_ERROR");
    assert_eq!(translate_error(0x1234), "UNKNOWN");
}

#[test]
fn blend_modes() {
    for mode in [
        BlendMode::NONE,
        BlendMode::BLEND,
        BlendMode::BLEND_PREMULTIPLIED,
        BlendMode::ADD,
        BlendMode::ADD_PREMULTIPLIED,
        BlendMode::MOD,
        BlendMode::MUL,
    ] {
        assert!(supports_blend_mode(mode), "{mode:?}");
    }
    let custom = |color_op, alpha_op| {
        BlendMode::compose_custom(BF::One, BF::One, color_op, BF::One, BF::Zero, alpha_op)
    };
    assert!(supports_blend_mode(custom(BO::Subtract, BO::Subtract)));
    assert!(supports_blend_mode(custom(BO::Maximum, BO::Maximum)));
    // The operations must match
    assert!(!supports_blend_mode(custom(BO::Add, BO::Subtract)));
    assert!(!supports_blend_mode(BlendMode::INVALID));

    assert_eq!(get_blend_func(Some(BF::OneMinusDstAlpha)), 0x0305);
    assert_eq!(get_blend_func(Some(BF::DstColor)), 0x0306);
    assert_eq!(get_blend_equation(Some(BO::RevSubtract)), 0x800B);
    assert_eq!(get_blend_equation(None), GL_INVALID_ENUM);
}

#[test]
fn formats() {
    assert_eq!(
        convert_format(PixelFormat::BGRA32),
        Some((GL_RGBA8, GL_BGRA, GL_UNSIGNED_BYTE))
    );
    assert_eq!(
        convert_format(PixelFormat::RGBX32),
        Some((GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE))
    );
    for f in [PixelFormat::INDEX8, PixelFormat::IYUV, PixelFormat::NV21] {
        assert_eq!(
            convert_format(f),
            Some((0x1909, GL_LUMINANCE, GL_UNSIGNED_BYTE))
        );
    }
    assert_eq!(convert_format(PixelFormat::RGB565), None);
    assert_eq!(convert_format(PixelFormat::P010), None);
}

#[test]
fn unpack_bounds() {
    let pixels = [0u8; 100];
    // 3 rows of 4 bytes, 10 bytes apart: 24 bytes
    assert!(unpack_source(&pixels, 76, 4, 3, 10, 1).is_ok());
    assert!(unpack_source(&pixels, 77, 4, 3, 10, 1).is_err());
    // Row length 0 is the width
    assert!(unpack_source(&pixels, 0, 10, 10, 0, 1).is_ok());
    assert!(unpack_source(&pixels, 0, 5, 10, 0, 2).is_ok());
    assert!(unpack_source(&pixels, 0, 5, 11, 0, 2).is_err());
    // Nothing to read
    assert!(unpack_source(&pixels, 100, 0, 5, 0, 4).is_ok());
    assert!(unpack_source(&pixels, 101, 0, 5, 0, 4).is_err());
}

#[test]
fn shader_sources() {
    assert!(shaders::source_of(Shader::None).is_none());
    let (vert, frag, version) = shaders::source_of(Shader::Solid).unwrap();
    assert!(vert.contains("gl_Position = gl_ModelViewProjectionMatrix * gl_Vertex;\n"));
    assert!(frag.ends_with("    gl_FragColor = v_color;\n}"));
    assert_eq!(version, None);
    let swizzles = [
        (Shader::Nv12Ra, ".ra;"),
        (Shader::Nv12Rg, ".rg;"),
        (Shader::Nv21Ra, ".ar;"),
        (Shader::Nv21Rg, ".gr;"),
    ];
    for (shader, swizzle) in swizzles {
        let (_, frag, _) = shaders::source_of(shader).unwrap();
        assert!(
            frag.contains(&format!("    yuv.yz = texture2D(tex1, tcoord){swizzle}\n")),
            "{shader:?}"
        );
    }
    for shader in [
        Shader::PalettePixelart,
        Shader::RgbPixelart,
        Shader::RgbaPixelart,
    ] {
        assert_eq!(
            shaders::source_of(shader).unwrap().2,
            Some("#version 130\n")
        );
    }
    let (_, frag, _) = shaders::source_of(Shader::Rgb).unwrap();
    assert!(frag.contains("    gl_FragColor.a = 1.0;\n"));
}

// ---------------------------------------------------------------------------
// With a context
// ---------------------------------------------------------------------------

/// The video drivers that may give an OpenGL context here.
fn gl_video_drivers() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["windows"]
    } else {
        let mut drivers = vec!["offscreen"];
        if std::env::var_os("DISPLAY").is_some() {
            drivers.push("x11");
        }
        drivers
    }
}

/// Video up on a driver; quit when dropped.
struct Video;

impl Video {
    fn init(driver: &str) -> Option<Video> {
        init::quit();
        hints::set(hints::VIDEO_DRIVER, driver).unwrap();
        if let Err(e) = init::init(InitFlags::VIDEO) {
            eprintln!("note: no {driver} video ({}); skipping", e.message());
            hints::reset(hints::VIDEO_DRIVER);
            return None;
        }
        Some(Video)
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        init::quit();
        hints::reset(hints::VIDEO_DRIVER);
    }
}

/// An OpenGL renderer for a window of a video driver. The renderer is
/// declared first, so it goes before the window and the video subsystem.
struct GlTest {
    r: Renderer,
    window: Window,
    driver: &'static str,
    _video: Video,
}

impl GlTest {
    fn new(driver: &'static str) -> Option<GlTest> {
        let video = Video::init(driver)?;
        let window = match Window::create("opengl renderer", W, H, WindowFlags::default()) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("note: no {driver} window ({}); skipping", e.message());
                return None;
            }
        };
        match Renderer::for_window(&window, Some(OPENGL_RENDERER)) {
            Ok(r) => Some(GlTest {
                r,
                window,
                driver,
                _video: video,
            }),
            Err(e) => {
                eprintln!(
                    "note: no OpenGL renderer on {driver} ({}); skipping",
                    e.message()
                );
                window.destroy();
                None
            }
        }
    }
}

/// Each driver's OpenGL renderer that can be made.
fn for_each_gl(mut f: impl FnMut(&mut GlTest)) {
    for driver in gl_video_drivers() {
        if let Some(mut t) = GlTest::new(driver) {
            f(&mut t);
        }
    }
}

/// A software renderer of the same size.
fn software() -> Renderer {
    Renderer::software(Surface::new(W, H, PixelFormat::ARGB8888).unwrap()).unwrap()
}

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

/// Read `rect` of the current target (`None`: all of it) as RGBA32.
fn read(r: &mut Renderer, rect: Option<&Rect>) -> (i32, i32, Vec<u8>) {
    rgba(&r.read_pixels(rect).unwrap())
}

/// Compare two images: at most `max_bad` pixels may have a channel off by
/// more than `tolerance` (alpha is only compared with `with_alpha`).
fn compare(
    what: &str,
    a: &(i32, i32, Vec<u8>),
    b: &(i32, i32, Vec<u8>),
    tolerance: u8,
    max_bad: usize,
    with_alpha: bool,
) {
    assert_eq!((a.0, a.1), (b.0, b.1), "{what}: sizes");
    let channels = if with_alpha { 4 } else { 3 };
    let mut bad = 0;
    let mut first = None;
    for (i, (pa, pb)) in a.2.chunks(4).zip(b.2.chunks(4)).enumerate() {
        if (0..channels).any(|c| pa[c].abs_diff(pb[c]) > tolerance) {
            bad += 1;
            if first.is_none() {
                first = Some((i as i32 % a.0, i as i32 / a.0, pa.to_vec(), pb.to_vec()));
            }
        }
    }
    assert!(
        bad <= max_bad,
        "{what}: {bad} pixels differ (first at {:?}: opengl vs software)",
        first
    );
}

/// Draw a scene with the OpenGL renderer and the software renderer and
/// compare the results.
fn compare_scene(
    t: &mut GlTest,
    what: &str,
    tolerance: u8,
    max_bad: usize,
    scene: impl Fn(&mut Renderer) -> Result<()>,
) {
    let what = format!("{what} ({})", t.driver);
    scene(&mut t.r).unwrap_or_else(|e| panic!("{what}: opengl: {}", e.message()));
    let gl = read(&mut t.r, None);
    let mut sw = software();
    scene(&mut sw).unwrap_or_else(|e| panic!("{what}: software: {}", e.message()));
    let soft = read(&mut sw, None);
    compare(&what, &gl, &soft, tolerance, max_bad, false);
}

/// A 16x16 texture of a pattern with varying alpha.
fn pattern_surface(format: PixelFormat) -> Surface<'static> {
    let mut s = Surface::new(16, 16, format).unwrap();
    for y in 0..16 {
        for x in 0..16 {
            let c = Color::new(
                (x * 16) as u8,
                (y * 16) as u8,
                ((x + y) * 8) as u8,
                if (x + y) % 3 == 0 {
                    255
                } else {
                    (x * 8 + y * 7) as u8
                },
            );
            s.write_pixel(x, y, c).unwrap();
        }
    }
    s
}

fn background(r: &mut Renderer) -> Result<()> {
    r.set_draw_blend_mode(BlendMode::NONE)?;
    r.set_draw_color(10, 20, 30, 255);
    r.clear()?;
    r.set_draw_color(90, 160, 200, 255);
    r.render_fill_rect(Some(&FRect::new(0.0, 24.0, 64.0, 24.0)))
}

#[test]
fn properties_and_formats() {
    let _l = crate::test_support::test_lock();
    for_each_gl(|t| {
        let r = &mut t.r;
        assert_eq!(r.name(), OPENGL_RENDERER);
        let props = r.properties();
        assert!(
            props
                .get_number(PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER)
                .unwrap_or(0)
                > 0
        );
        assert_eq!(
            props.get_bool(PROP_RENDERER_TEXTURE_WRAPPING_BOOLEAN),
            Some(true)
        );
        let formats = r.texture_formats().to_vec();
        assert!(formats.starts_with(&[PixelFormat::ARGB8888, PixelFormat::ABGR8888]));
        let names: Vec<&str> = formats.iter().map(|f| f.name()).collect();
        eprintln!("{}: OpenGL renderer formats {names:?}", t.driver);
        assert_eq!(r.output_size().unwrap(), (W, H));

        let tex = r
            .create_texture(PixelFormat::ARGB8888, TextureAccess::Static, 20, 10)
            .unwrap();
        let tprops = r.texture_properties(tex).unwrap();
        assert!(
            tprops
                .get_number(PROP_TEXTURE_OPENGL_TEXTURE_NUMBER)
                .unwrap_or(0)
                > 0
        );
        assert_eq!(
            tprops.get_number(PROP_TEXTURE_OPENGL_TEXTURE_TARGET_NUMBER),
            Some(GL_TEXTURE_2D as i64)
        );
        assert_eq!(tprops.get_float(PROP_TEXTURE_OPENGL_TEX_W_FLOAT), Some(1.0));
        assert_eq!(tprops.get_float(PROP_TEXTURE_OPENGL_TEX_H_FLOAT), Some(1.0));
        if formats.contains(&PixelFormat::NV12) {
            let nv = r
                .create_texture(PixelFormat::NV12, TextureAccess::Static, 8, 8)
                .unwrap();
            let p = r.texture_properties(nv).unwrap();
            assert!(
                p.get_number(PROP_TEXTURE_OPENGL_TEXTURE_UV_NUMBER)
                    .unwrap_or(0)
                    > 0
            );
        }

        // Unsupported formats go through a native texture
        let other = r
            .create_texture(PixelFormat::RGB565, TextureAccess::Static, 4, 4)
            .unwrap();
        assert!(r.texture_properties(other).is_ok());
        // Palettized textures can't be targets (the front end's check)
        assert!(r
            .create_texture(PixelFormat::INDEX8, TextureAccess::Target, 4, 4)
            .is_err());

        r.set_vsync(0).unwrap();
        assert_eq!(r.vsync(), 0);
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.present().unwrap();
    });
}

#[test]
fn primitives_match_software() {
    let _l = crate::test_support::test_lock();
    for_each_gl(|t| {
        compare_scene(t, "clear and fill", 0, 0, |r| {
            r.set_draw_color(10, 20, 30, 255);
            r.clear()?;
            r.set_draw_color(200, 100, 50, 255);
            r.render_fill_rect(Some(&FRect::new(8.0, 8.0, 32.0, 24.0)))?;
            r.set_draw_color(0, 255, 0, 255);
            r.render_fill_rects(&[
                FRect::new(-5.0, -5.0, 10.0, 10.0),
                FRect::new(50.0, 40.0, 30.0, 30.0),
                FRect::new(20.0, 30.0, 1.0, 1.0),
            ])?;
            r.set_draw_color(255, 255, 255, 255);
            r.render_rect(Some(&FRect::new(2.0, 30.0, 12.0, 10.0)))
        });
        compare_scene(t, "points", 0, 0, |r| {
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
            r.set_draw_color(0, 128, 255, 255);
            r.render_point(63.0, 47.0)
        });
        // (lines are points for the OpenGL renderer, the software renderer
        // draws them itself: they may part ways where Bresenham ties)
        compare_scene(t, "lines", 0, 4, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            r.set_draw_color(255, 255, 0, 255);
            r.render_line(0.0, 0.0, 63.0, 0.0)?;
            r.render_line(5.0, 3.0, 5.0, 40.0)?;
            r.render_line(10.0, 10.0, 40.0, 30.0)?;
            r.render_lines(&[
                FPoint { x: 50.0, y: 5.0 },
                FPoint { x: 60.0, y: 15.0 },
                FPoint { x: 50.0, y: 25.0 },
            ])
        });
        compare_scene(t, "viewport and clip", 0, 0, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            r.set_viewport(Some(&Rect::new(10, 6, 40, 30)))?;
            r.set_draw_color(255, 0, 0, 255);
            r.render_fill_rect(None)?;
            r.set_clip_rect(Some(&Rect::new(5, 5, 12, 8)))?;
            r.set_draw_color(0, 0, 255, 255);
            r.render_fill_rect(Some(&FRect::new(0.0, 0.0, 30.0, 30.0)))?;
            r.render_point(6.0, 6.0)?;
            r.set_clip_rect(None)?;
            r.set_draw_color(0, 255, 0, 255);
            r.render_point(39.0, 29.0)?;
            r.set_viewport(None)?;
            r.render_point(0.0, 47.0)
        });
        compare_scene(t, "blend modes", 2, 0, |r| {
            background(r)?;
            for (i, mode) in [
                BlendMode::BLEND,
                BlendMode::ADD,
                BlendMode::MOD,
                BlendMode::MUL,
                BlendMode::BLEND_PREMULTIPLIED,
            ]
            .into_iter()
            .enumerate()
            {
                r.set_draw_blend_mode(mode)?;
                r.set_draw_color(220, 90, 40, 128);
                r.render_fill_rect(Some(&FRect::new(i as f32 * 12.0, 12.0, 10.0, 30.0)))?;
            }
            Ok(())
        });
        compare_scene(t, "geometry", 0, 0, |r| {
            r.set_draw_color(0, 0, 0, 255);
            r.clear()?;
            let c = FColor::new(0.2, 0.6, 1.0, 1.0);
            let v = |x: f32, y: f32| Vertex {
                position: FPoint { x, y },
                color: c,
                tex_coord: FPoint::default(),
            };
            r.render_geometry(
                None,
                &[v(4.0, 4.0), v(36.0, 4.0), v(36.0, 28.0), v(4.0, 28.0)],
                Some(&[0, 1, 2, 0, 2, 3]),
            )
        });
    });
}

#[test]
fn textures_match_software() {
    let _l = crate::test_support::test_lock();
    for_each_gl(|t| {
        let copy = |r: &mut Renderer, format, blend, modulate: bool| -> Result<()> {
            background(r)?;
            let tex = r.create_texture_from_surface(&mut pattern_surface(format))?;
            r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
            r.set_texture_blend_mode(tex, blend)?;
            if modulate {
                r.set_texture_color_mod(tex, 128, 255, 64)?;
                r.set_texture_alpha_mod(tex, 200)?;
            }
            r.render_texture(tex, None, Some(&FRect::new(5.0, 7.0, 16.0, 16.0)))?;
            r.render_texture(
                tex,
                Some(&FRect::new(4.0, 4.0, 8.0, 8.0)),
                Some(&FRect::new(40.0, 20.0, 8.0, 8.0)),
            )
        };
        compare_scene(t, "copy", 0, 0, |r| {
            copy(r, PixelFormat::ARGB8888, BlendMode::NONE, false)
        });
        compare_scene(t, "copy ABGR", 0, 0, |r| {
            copy(r, PixelFormat::ABGR8888, BlendMode::NONE, false)
        });
        compare_scene(t, "copy XRGB", 0, 0, |r| {
            copy(r, PixelFormat::XRGB8888, BlendMode::BLEND, false)
        });
        compare_scene(t, "copy blended", 2, 0, |r| {
            copy(r, PixelFormat::ARGB8888, BlendMode::BLEND, false)
        });
        compare_scene(t, "copy added", 2, 0, |r| {
            copy(r, PixelFormat::ARGB8888, BlendMode::ADD, false)
        });
        compare_scene(t, "copy modulated", 3, 0, |r| {
            copy(r, PixelFormat::ARGB8888, BlendMode::BLEND, true)
        });
        // (opaque: the software renderer's rotated copies multiply the color
        // by the alpha under BlendMode::NONE)
        compare_scene(t, "copy flipped", 0, 0, |r| {
            background(r)?;
            let tex = r.create_texture_from_surface(&mut pattern_surface(PixelFormat::XRGB8888))?;
            r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
            r.set_texture_blend_mode(tex, BlendMode::NONE)?;
            r.render_texture_rotated(
                tex,
                None,
                Some(&FRect::new(30.0, 10.0, 16.0, 16.0)),
                0.0,
                None,
                FlipMode::Horizontal,
            )
        });
        // (nearest scaling: the stretch blitter and GL's sampler may pick
        // different texels where the source edges fall on pixel centers)
        compare_scene(t, "copy scaled", 0, 32, |r| {
            background(r)?;
            let tex = r.create_texture_from_surface(&mut pattern_surface(PixelFormat::ARGB8888))?;
            r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
            r.set_texture_blend_mode(tex, BlendMode::NONE)?;
            r.render_texture(tex, None, Some(&FRect::new(2.0, 2.0, 32.0, 32.0)))
        });
        compare_scene(t, "streaming", 0, 0, |r| {
            background(r)?;
            let tex = r.create_texture(PixelFormat::ARGB8888, TextureAccess::Streaming, 8, 6)?;
            {
                let mut lock = r.lock_texture(tex, Some(&Rect::new(2, 1, 4, 3)))?;
                let pitch = lock.pitch() as usize;
                let pixels = lock.pixels();
                for y in 0..3 {
                    for x in 0..4 {
                        let p = &mut pixels[y * pitch + x * 4..][..4];
                        p.copy_from_slice(&[(x * 60) as u8, (y * 80) as u8, 200, 255]);
                    }
                }
            }
            r.set_texture_blend_mode(tex, BlendMode::NONE)?;
            r.render_texture(
                tex,
                Some(&FRect::new(2.0, 1.0, 4.0, 3.0)),
                Some(&FRect::new(10.0, 10.0, 4.0, 3.0)),
            )?;
            let pixels: Vec<u8> = (0..8 * 6 * 4).map(|i| (i * 5) as u8).collect();
            r.update_texture(tex, None, &pixels, 8 * 4)?;
            r.render_texture(tex, None, Some(&FRect::new(30.0, 30.0, 8.0, 6.0)))
        });
        compare_scene(t, "palette", 0, 0, |r| {
            background(r)?;
            let mut palette = Palette::new(256)?;
            let colors: Vec<Color> = (0..256)
                .map(|i| Color::new(i as u8, (255 - i) as u8, (i * 3) as u8, 255))
                .collect();
            palette.set_colors(0, &colors)?;
            let palette = share_palette(palette);
            let tex = r.create_texture_with(&TextureCreateInfo {
                format: PixelFormat::INDEX8,
                access: TextureAccess::Static,
                width: 8,
                height: 8,
                palette: Some(palette),
                ..TextureCreateInfo::default()
            })?;
            let indices: Vec<u8> = (0..64).map(|i| (i * 37 % 256) as u8).collect();
            r.update_texture(tex, None, &indices, 8)?;
            r.set_texture_scale_mode(tex, ScaleMode::Nearest)?;
            r.render_texture(tex, None, Some(&FRect::new(20.0, 12.0, 8.0, 8.0)))?;
            r.render_texture(tex, None, Some(&FRect::new(40.0, 30.0, 8.0, 8.0)))
        });
        // (GL converts YUV with float shaders, the software path with
        // fixed point)
        compare_scene(t, "yuv", 4, 0, |r| {
            background(r)?;
            let (w, h) = (16usize, 12usize);
            let y: Vec<u8> = (0..w * h).map(|i| (16 + i % 200) as u8).collect();
            // (in gamut: the software conversion wraps below -128, as the
            // clampU8() table upstream does)
            let u: Vec<u8> = (0..w * h / 4).map(|i| (96 + i * 3 % 64) as u8).collect();
            let v: Vec<u8> = (0..w * h / 4).map(|i| (160 - i * 2 % 64) as u8).collect();
            let iyuv = r.create_texture(PixelFormat::IYUV, TextureAccess::Static, 16, 12)?;
            r.update_yuv_texture(iyuv, None, &y, 16, &u, 8, &v, 8)?;
            r.set_texture_scale_mode(iyuv, ScaleMode::Nearest)?;
            r.render_texture(iyuv, None, Some(&FRect::new(2.0, 2.0, 16.0, 12.0)))?;

            let uv: Vec<u8> = u.iter().zip(&v).flat_map(|(&u, &v)| [u, v]).collect();
            let nv12 = r.create_texture(PixelFormat::NV12, TextureAccess::Static, 16, 12)?;
            r.update_nv_texture(nv12, None, &y, 16, &uv, 16)?;
            r.set_texture_scale_mode(nv12, ScaleMode::Nearest)?;
            r.render_texture(nv12, None, Some(&FRect::new(24.0, 2.0, 16.0, 12.0)))?;

            // The whole planes in one buffer
            let mut all = y.clone();
            all.extend_from_slice(&u);
            all.extend_from_slice(&v);
            let yv12 = r.create_texture(PixelFormat::YV12, TextureAccess::Static, 16, 12)?;
            r.update_texture(yv12, None, &all, 16)?;
            r.set_texture_scale_mode(yv12, ScaleMode::Nearest)?;
            r.render_texture(yv12, None, Some(&FRect::new(2.0, 20.0, 16.0, 12.0)))?;

            let mut all = y.clone();
            all.extend_from_slice(&uv);
            let nv21 = r.create_texture(PixelFormat::NV21, TextureAccess::Streaming, 16, 12)?;
            r.update_texture(nv21, None, &all, 16)?;
            r.set_texture_scale_mode(nv21, ScaleMode::Nearest)?;
            r.render_texture(nv21, None, Some(&FRect::new(24.0, 20.0, 16.0, 12.0)))
        });
    });
}

#[test]
fn render_targets_and_read_pixels() {
    let _l = crate::test_support::test_lock();
    for_each_gl(|t| {
        let draw_target = |r: &mut Renderer| -> Result<Texture> {
            let target = r.create_texture(PixelFormat::ARGB8888, TextureAccess::Target, 32, 24)?;
            r.set_render_target(Some(target))?;
            r.set_draw_color(255, 0, 0, 128);
            r.clear()?;
            r.set_draw_color(0, 0, 255, 255);
            r.render_fill_rect(Some(&FRect::new(2.0, 3.0, 10.0, 5.0)))?;
            r.set_draw_color(0, 255, 0, 255);
            r.render_point(31.0, 23.0)?;
            r.set_clip_rect(Some(&Rect::new(20, 0, 5, 30)))?;
            r.render_fill_rect(None)?;
            Ok(target)
        };

        // The target's pixels, alpha included
        let target = draw_target(&mut t.r).unwrap();
        let gl = read(&mut t.r, None);
        let gl_part = read(&mut t.r, Some(&Rect::new(1, 2, 12, 7)));
        let mut sw = software();
        let sw_target = draw_target(&mut sw).unwrap();
        let soft = read(&mut sw, None);
        let soft_part = read(&mut sw, Some(&Rect::new(1, 2, 12, 7)));
        let what = format!("target ({})", t.driver);
        compare(&what, &gl, &soft, 0, 0, true);
        compare(&what, &gl_part, &soft_part, 0, 0, true);

        // Drawn on the window
        for (r, target) in [(&mut t.r, target), (&mut sw, sw_target)] {
            r.set_render_target(None).unwrap();
            background(r).unwrap();
            r.set_texture_blend_mode(target, BlendMode::BLEND).unwrap();
            r.set_texture_scale_mode(target, ScaleMode::Nearest)
                .unwrap();
            r.render_texture(target, None, Some(&FRect::new(16.0, 12.0, 32.0, 24.0)))
                .unwrap();
        }
        let gl = read(&mut t.r, None);
        let soft = read(&mut sw, None);
        compare(
            &format!("target drawn ({})", t.driver),
            &gl,
            &soft,
            2,
            0,
            false,
        );

        // A part of the window: the rows come out top-down
        let gl_part = read(&mut t.r, Some(&Rect::new(10, 20, 30, 9)));
        let soft_part = read(&mut sw, Some(&Rect::new(10, 20, 30, 9)));
        compare(
            &format!("window part ({})", t.driver),
            &gl_part,
            &soft_part,
            2,
            0,
            false,
        );

        // An ABGR target reads back as such
        let abgr =
            t.r.create_texture(PixelFormat::ABGR8888, TextureAccess::Target, 4, 4)
                .unwrap();
        t.r.set_render_target(Some(abgr)).unwrap();
        t.r.set_draw_color(1, 2, 3, 4);
        t.r.clear().unwrap();
        let s = t.r.read_pixels(None).unwrap();
        assert_eq!(s.format(), PixelFormat::ABGR8888);
        assert_eq!(s.read_pixel(3, 3).unwrap(), Color::new(1, 2, 3, 4));
        t.r.set_render_target(None).unwrap();
        t.r.destroy_texture(abgr);
        t.r.destroy_texture(target);
        t.r.present().unwrap();
    });
}

/// The window going first: the renderer's calls fail (without touching
/// GL, whose library may be gone) and it can still be dropped.
#[test]
fn window_destroyed_first() {
    let _l = crate::test_support::test_lock();
    for_each_gl(|t| {
        let tex =
            t.r.create_texture(PixelFormat::ARGB8888, TextureAccess::Streaming, 8, 8)
                .unwrap();
        t.r.render_texture(tex, None, None).unwrap();
        t.window.destroy();
        assert!(t.r.clear().is_err());
        assert!(t.r.present().is_err());
        assert!(t.r.read_pixels(None).is_err());
    });
}

/// With a debug context, GL errors are reported (`GL_ARB_debug_output` or
/// `glGetError()`): an external texture of the wrong target can't be
/// bound as a 2D texture.
#[test]
fn debug_context_reports_errors() {
    let _l = crate::test_support::test_lock();
    for driver in gl_video_drivers() {
        let Some(_video) = Video::init(driver) else {
            continue;
        };
        if gl::gl_set_attribute(GlAttr::ContextFlags, gl::GL_CONTEXT_DEBUG_FLAG).is_err() {
            continue;
        }
        let window = Window::create("debug", 16, 16, WindowFlags::default()).unwrap();
        let mut r = match Renderer::for_window(&window, Some(OPENGL_RENDERER)) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "note: no OpenGL debug renderer on {driver} ({}); skipping",
                    e.message()
                );
                continue;
            }
        };
        type GenTextures = unsafe extern "system" fn(i32, *mut u32);
        type BindTexture = unsafe extern "system" fn(u32, u32);
        const GL_TEXTURE_1D: u32 = 0x0DE0;
        // SAFETY: the types are the entry points'; the renderer's context
        // is current (it was just made).
        let name = unsafe {
            let gen: GenTextures = gl::gl_function("glGenTextures").unwrap();
            let bind: BindTexture = gl::gl_function("glBindTexture").unwrap();
            let mut name = 0;
            gen(1, &mut name);
            bind(GL_TEXTURE_1D, name);
            bind(GL_TEXTURE_1D, 0);
            name
        };
        let result = r.create_texture_with(&TextureCreateInfo {
            format: PixelFormat::ARGB8888,
            width: 4,
            height: 4,
            opengl_texture: Some(name),
            ..TextureCreateInfo::default()
        });
        let e = result.expect_err("binding a 1D texture as 2D fails");
        eprintln!("{driver}: {}", e.message());
        assert!(
            e.message().starts_with("glTexImage2D(): SDL_render_gl.c ("),
            "{}",
            e.message()
        );
        assert!(e.message().contains("GL_CreateTexture"), "{}", e.message());

        // The renderer goes on
        r.set_draw_color(9, 8, 7, 255);
        r.clear().unwrap();
        let s = r.read_pixels(Some(&Rect::new(0, 0, 1, 1))).unwrap();
        let c = s.read_pixel(0, 0).unwrap();
        assert_eq!((c.r, c.g, c.b), (9, 8, 7));
        drop(r);
        window.destroy();
    }
}

/// Without framebuffer objects the renderer can't be made (as with GDI
/// Generic's OpenGL 1.1): the window is put back as it was, and the
/// driver list falls back to the next driver.
#[test]
fn fallback_without_framebuffer_objects() {
    let _l = crate::test_support::test_lock();
    for driver in gl_video_drivers() {
        let Some(_video) = Video::init(driver) else {
            continue;
        };
        if gl::gl_load_library(None).is_err() {
            eprintln!("note: no OpenGL library on {driver}; skipping");
            continue;
        }
        gl::gl_unload_library();
        hints::set("GL_EXT_framebuffer_object", "0").unwrap();
        let window = Window::create("fallback", 16, 16, WindowFlags::default()).unwrap();
        match Renderer::for_window(&window, Some(OPENGL_RENDERER)) {
            Ok(_) => panic!("{driver}: an OpenGL renderer without render targets"),
            Err(e) => assert!(
                e.message()
                    == "Can't create render targets, GL_EXT_framebuffer_object not available"
                    || e.message().starts_with("Couldn't load GL function")
                    || e.message().contains("context"),
                "{driver}: {}",
                e.message()
            ),
        }
        assert!(!window.flags().unwrap().contains(WindowFlags::OPENGL));
        let mut r = Renderer::for_window(&window, None).unwrap();
        assert_ne!(r.name(), OPENGL_RENDERER);
        r.set_draw_color(1, 2, 3, 255);
        r.clear().unwrap();
        r.present().unwrap();
        drop(r);
        hints::reset("GL_EXT_framebuffer_object");
        window.destroy();
    }
}
