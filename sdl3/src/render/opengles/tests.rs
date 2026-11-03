// Tests of the OpenGL ES 1.1 renderer: scenes drawn through a real OpenGL
// ES 1.1 context and through the software renderer, compared pixel by
// pixel. They run on the offscreen driver's EGL, or on the video driver
// SDL_VIDEO_DRIVER names: the Linux CI runs them on X11 (Xvfb) with ANGLE's
// OpenGL ES 1 on SwiftShader (Chrome's libEGL and libGLESv2), as Mesa's
// Ubuntu builds have no OpenGL ES 1. Where there is no OpenGL ES 1 context
// (capability `gles1`), or no EGL at all (`egl`), the GL tests skip; the
// Android and UIKit apps (`tests/android_gles1.rs`, `tests/uikit_gles1.rs`)
// draw with the renderer on their windows.

use super::*;
use crate::hints;
use crate::init::{self, InitFlags};
use crate::render::{
    setup_renderer_colorspace, Renderer, RendererCreateInfo, Vertex,
    PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER, PROP_RENDERER_OUTPUT_COLORSPACE_NUMBER, RENDER_DRIVERS,
};
use crate::video::pixels::Palette;
use crate::video::surface::share_palette;

const W: i32 = 64;
const H: i32 = 48;

/// Video up on the offscreen driver (or the one `SDL_VIDEO_DRIVER` names);
/// quit when dropped.
struct Video;

impl Video {
    fn init() -> Video {
        init::quit();
        if std::env::var_os("SDL_VIDEO_DRIVER").is_none() {
            hints::set(hints::VIDEO_DRIVER, "offscreen").unwrap();
        }
        init::init(InitFlags::VIDEO).unwrap();
        Video
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        init::quit();
        hints::reset(hints::VIDEO_DRIVER);
    }
}

/// An OpenGL ES 1.1 renderer for `window`, made as `SDL_CreateRenderer()`
/// makes it where the driver list has it, and otherwise (where upstream
/// doesn't build it) from the backend as the front end makes it.
fn create(window: &Window, info: &RendererCreateInfo) -> Result<Renderer> {
    if RENDER_DRIVERS.contains(&GLES_RENDERER) {
        let info = RendererCreateInfo {
            name: Some(GLES_RENDERER.to_owned()),
            ..info.clone()
        };
        return Renderer::for_window_with(window, &info);
    }
    let backend = GlesRenderer::for_window(*window, setup_renderer_colorspace(info))?;
    Renderer::finish_create(
        Box::new(backend),
        PixelFormat::UNKNOWN,
        (0, 0),
        info,
        0,
        Some(*window),
    )
}

/// The capability a missing OpenGL ES 1.1 renderer is: `gles1` where the
/// OpenGL ES 2 renderer can be made (EGL is there, OpenGL ES 1 isn't),
/// `egl` otherwise.
fn missing_capability() -> &'static str {
    let window = Window::create("gles2", 8, 8, WindowFlags::default()).unwrap();
    let gles2 = Renderer::for_window(&window, Some("opengles2"));
    let capability = if gles2.is_ok() { "gles1" } else { "egl" };
    drop(gles2);
    window.destroy();
    capability
}

/// A window with an OpenGL ES 1.1 renderer, or `None` (with the skip
/// reported) when there is no OpenGL ES 1.
fn gles_renderer_with(w: i32, h: i32, info: &RendererCreateInfo) -> Option<(Window, Renderer)> {
    let window = Window::create("gles1", w, h, WindowFlags::default()).unwrap();
    match create(&window, info) {
        Ok(r) => Some((window, r)),
        Err(e) => {
            window.destroy();
            crate::test_support::skip(
                missing_capability(),
                format_args!("no OpenGL ES 1 renderer ({})", e.message()),
            );
            None
        }
    }
}

fn gles_renderer(w: i32, h: i32) -> Option<(Window, Renderer)> {
    gles_renderer_with(w, h, &RendererCreateInfo::default())
}

/// A software renderer drawing into an RGBA32 surface (the format the
/// renderer reads the window back in).
fn software_renderer(w: i32, h: i32) -> Renderer {
    Renderer::software(Surface::new(w, h, PixelFormat::RGBA32).unwrap()).unwrap()
}

/// The pixels of a surface, row by row without padding.
fn rows(s: &Surface<'_>) -> Vec<u8> {
    let bpp = s.format().bytes_per_pixel() as usize;
    let row = s.width() as usize * bpp;
    let pixels = s.raw_pixels().unwrap();
    (0..s.height() as usize)
        .flat_map(|y| pixels[y * s.pitch() as usize..][..row].to_vec())
        .collect()
}

/// A pixel where two images differ: its position and both values.
type Difference = ((i32, i32), Vec<u8>, Vec<u8>);

/// The pixels where two RGBA32 images differ by more than `tol` in a
/// channel (the color channels only when `rgb_only`).
fn differences(gl: &Surface<'_>, sw: &Surface<'_>, tol: u8, rgb_only: bool) -> Vec<Difference> {
    let (a, b) = (rows(gl), rows(sw));
    let w = gl.width() as usize;
    let channels = if rgb_only { 3 } else { 4 };
    a.chunks(4)
        .zip(b.chunks(4))
        .enumerate()
        .filter(|(_, (pa, pb))| {
            pa.iter()
                .zip(*pb)
                .take(channels)
                .any(|(x, y)| x.abs_diff(*y) > tol)
        })
        .map(|(i, (pa, pb))| (((i % w) as i32, (i / w) as i32), pa.to_vec(), pb.to_vec()))
        .collect()
}

/// Compare two images, each channel within `tol` (both are converted to
/// RGBA32 first).
fn assert_close_with(what: &str, gl: &Surface<'_>, sw: &Surface<'_>, tol: u8, rgb_only: bool) {
    let gl = gl.convert(PixelFormat::RGBA32).unwrap();
    let sw = sw.convert(PixelFormat::RGBA32).unwrap();
    assert_eq!(
        (gl.width(), gl.height()),
        (sw.width(), sw.height()),
        "{what}: sizes"
    );
    let bad = differences(&gl, &sw, tol, rgb_only);
    if let Some(((x, y), pa, pb)) = bad.first() {
        panic!(
            "{what}: {} pixels differ by more than {tol}; first at ({x}, {y}): gl {pa:?} sw {pb:?}",
            bad.len()
        );
    }
}

fn assert_close(what: &str, gl: &Surface<'_>, sw: &Surface<'_>, tol: u8) {
    assert_close_with(what, gl, sw, tol, false);
}

/// Draw `scene` with the GLES renderer and the software renderer, and
/// compare the results (`tol` per channel).
fn compare(what: &str, r: &mut Renderer, tol: u8, rgb_only: bool, scene: impl Fn(&mut Renderer)) {
    scene(r);
    let gl = r.read_pixels(None).unwrap();
    let mut sw = software_renderer(W, H);
    scene(&mut sw);
    let sw = sw.read_pixels(None).unwrap();
    assert_close_with(what, &gl, &sw, tol, rgb_only);
}

/// An RGBA pattern with varying alpha.
fn pattern(w: i32, h: i32) -> Vec<u8> {
    let mut p = Vec::new();
    for y in 0..h {
        for x in 0..w {
            p.extend_from_slice(&[
                (x * 255 / (w - 1)) as u8,
                (y * 255 / (h - 1)) as u8,
                ((x + y) * 13 % 256) as u8,
                if (x / 4 + y / 4) % 2 == 0 {
                    255
                } else {
                    96 + x as u8
                },
            ]);
        }
    }
    p
}

/// A static texture of `format` with [`pattern`]'s pixels (given as
/// RGBA32 and converted).
fn pattern_texture(r: &mut Renderer, format: PixelFormat, w: i32, h: i32) -> Texture {
    let mut src = Surface::new(w, h, PixelFormat::RGBA32).unwrap();
    let pitch = src.pitch() as usize;
    let p = pattern(w, h);
    let pixels = src.pixels_mut().unwrap();
    for y in 0..h as usize {
        pixels[y * pitch..][..w as usize * 4]
            .copy_from_slice(&p[y * w as usize * 4..][..w as usize * 4]);
    }
    let converted = src.convert(format).unwrap();
    let t = r
        .create_texture(format, TextureAccess::Static, w, h)
        .unwrap();
    r.update_texture(t, None, converted.raw_pixels().unwrap(), converted.pitch())
        .unwrap();
    t
}

/// Whether the renderer's context has `extension` (made current first).
fn has_extension(r: &mut Renderer, extension: &str) -> bool {
    // (a flush makes the renderer's context current)
    r.set_draw_color(0, 0, 0, 255);
    r.clear().unwrap();
    r.flush().unwrap();
    gl::gl_extension_supported(extension)
}

// ---------------------------------------------------------------------------
// Without a context
// ---------------------------------------------------------------------------

#[test]
fn blend_tables_follow_upstream() {
    use BlendFactor as F;
    use BlendOperation as O;
    let factors = [
        (F::Zero, GL_ZERO),
        (F::One, GL_ONE),
        (F::SrcColor, GL_SRC_COLOR),
        (F::OneMinusSrcColor, GL_ONE_MINUS_SRC_COLOR),
        (F::SrcAlpha, GL_SRC_ALPHA),
        (F::OneMinusSrcAlpha, GL_ONE_MINUS_SRC_ALPHA),
        (F::DstColor, GL_DST_COLOR),
        (F::OneMinusDstColor, GL_ONE_MINUS_DST_COLOR),
        (F::DstAlpha, GL_DST_ALPHA),
        (F::OneMinusDstAlpha, GL_ONE_MINUS_DST_ALPHA),
    ];
    for (factor, gl) in factors {
        assert_eq!(get_blend_func(Some(factor)), gl, "{factor:?}");
    }
    assert_eq!(get_blend_func(None), GL_INVALID_ENUM);
    let operations = [
        (O::Add, 0x8006),
        (O::Subtract, 0x800A),
        (O::RevSubtract, 0x800B),
        (O::Minimum, 0x8007),
        (O::Maximum, 0x8008),
    ];
    for (operation, gl) in operations {
        assert_eq!(get_blend_equation(Some(operation)), gl, "{operation:?}");
    }
    assert_eq!(get_blend_equation(None), GL_INVALID_ENUM);
}

#[test]
fn error_names() {
    let e = |code| gles_set_error("glTexImage2D()", code).message().to_owned();
    assert_eq!(e(0), "glTexImage2D(): GL_NO_ERROR");
    assert_eq!(e(0x0500), "glTexImage2D(): GL_INVALID_ENUM");
    assert_eq!(e(0x0501), "glTexImage2D(): GL_INVALID_VALUE");
    assert_eq!(e(0x0502), "glTexImage2D(): GL_INVALID_OPERATION");
    assert_eq!(e(0x0503), "glTexImage2D(): GL_STACK_OVERFLOW");
    assert_eq!(e(0x0504), "glTexImage2D(): GL_STACK_UNDERFLOW");
    assert_eq!(e(0x0505), "glTexImage2D(): GL_OUT_OF_MEMORY");
    assert_eq!(e(0x1234), "glTexImage2D(): UNKNOWN");
}

#[test]
fn skips_without_a_gl_driver() {
    let _l = crate::test_support::test_lock();
    // The dummy driver has no GL: the renderer can't be created, and the
    // window and the attributes are put back the way they were.
    init::quit();
    // (over SDL_VIDEO_DRIVER too)
    hints::set_with_priority(
        hints::VIDEO_DRIVER,
        Some("dummy"),
        hints::Priority::Override,
    )
    .unwrap();
    init::init(InitFlags::VIDEO).unwrap();
    let window = Window::create("no gl", 16, 16, WindowFlags::default()).unwrap();
    let before = (
        gl::gl_get_attribute(GlAttr::ContextProfileMask).unwrap(),
        gl::gl_get_attribute(GlAttr::ContextMajorVersion).unwrap(),
        gl::gl_get_attribute(GlAttr::ContextMinorVersion).unwrap(),
    );
    assert!(create(&window, &RendererCreateInfo::default()).is_err());
    assert!(!window.flags().unwrap().contains(WindowFlags::OPENGL));
    let after = (
        gl::gl_get_attribute(GlAttr::ContextProfileMask).unwrap(),
        gl::gl_get_attribute(GlAttr::ContextMajorVersion).unwrap(),
        gl::gl_get_attribute(GlAttr::ContextMinorVersion).unwrap(),
    );
    assert_eq!(after, before);
    window.destroy();
    init::quit();
    hints::reset(hints::VIDEO_DRIVER);
}

// ---------------------------------------------------------------------------
// With an OpenGL ES 1.1 context
// ---------------------------------------------------------------------------

#[test]
#[cfg_attr(
    target_os = "emscripten",
    ignore = "no EGL on Emscripten's offscreen driver"
)]
fn gles_renderer_basics() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((window, mut r)) = gles_renderer(W, H) else {
        return;
    };
    assert_eq!(r.name(), GLES_RENDERER);
    // (the window was made an OpenGL ES 1.1 one)
    assert!(window.flags().unwrap().contains(WindowFlags::OPENGL));
    assert_eq!(
        gl::gl_get_attribute(GlAttr::ContextProfileMask).unwrap(),
        gl::GL_CONTEXT_PROFILE_ES
    );
    assert_eq!(
        gl::gl_get_attribute(GlAttr::ContextMajorVersion).unwrap(),
        1
    );
    assert_eq!(
        gl::gl_get_attribute(GlAttr::ContextMinorVersion).unwrap(),
        1
    );
    assert_eq!(r.output_size().unwrap(), (W, H));
    let props = r.properties();
    assert!(
        props
            .get_number(PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER)
            .unwrap()
            >= 64
    );
    assert_eq!(r.texture_formats(), &[PixelFormat::RGBA32]);
    // (other formats through a native texture)
    let t = r
        .create_texture(PixelFormat::ARGB8888, TextureAccess::Static, 8, 8)
        .unwrap();
    r.destroy_texture(t);

    r.set_draw_color(1, 2, 3, 255);
    r.clear().unwrap();
    r.present().unwrap();
    r.set_vsync(0).unwrap();
    assert_eq!(r.vsync(), 0);
    let s = r.read_pixels(Some(&Rect::new(3, 4, 2, 2))).unwrap();
    assert_eq!(s.format(), PixelFormat::RGBA32);
    assert_eq!(s.read_pixel(1, 1).unwrap(), Color::new(1, 2, 3, 255));

    // The window can go first.
    window.destroy();
    drop(r);
}

/// The custom blend modes are those the context's `GL_OES_blend_*` and
/// `GL_EXT_blend_minmax` extensions draw (`GLES_SupportsBlendMode()`), and
/// they draw as the extensions say.
#[test]
#[cfg_attr(
    target_os = "emscripten",
    ignore = "no EGL on Emscripten's offscreen driver"
)]
fn gles_custom_blend_modes() {
    use BlendFactor as F;
    use BlendOperation as O;
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = gles_renderer(W, H) else {
        return;
    };
    let func_separate = has_extension(&mut r, "GL_OES_blend_func_separate");
    let equation_separate = has_extension(&mut r, "GL_OES_blend_equation_separate");
    let subtract = has_extension(&mut r, "GL_OES_blend_subtract");
    let minmax = has_extension(&mut r, "GL_EXT_blend_minmax");
    println!(
        "func_separate={func_separate} equation_separate={equation_separate} subtract={subtract} minmax={minmax}"
    );
    let modes = [
        // (the same for color and alpha)
        (
            BlendMode::compose_custom(F::One, F::One, O::Add, F::One, F::One, O::Add),
            true,
        ),
        (
            BlendMode::compose_custom(
                F::One,
                F::One,
                O::RevSubtract,
                F::One,
                F::One,
                O::RevSubtract,
            ),
            subtract,
        ),
        (
            BlendMode::compose_custom(F::One, F::One, O::Minimum, F::One, F::One, O::Minimum),
            subtract && minmax,
        ),
        (
            BlendMode::compose_custom(F::One, F::One, O::Maximum, F::One, F::One, O::Maximum),
            subtract && minmax,
        ),
        // (separate factors)
        (
            BlendMode::compose_custom(
                F::SrcAlpha,
                F::OneMinusSrcAlpha,
                O::Add,
                F::Zero,
                F::One,
                O::Add,
            ),
            func_separate,
        ),
        // (separate operations: the scenes' custom blend mode)
        (
            BlendMode::compose_custom(F::One, F::One, O::RevSubtract, F::Zero, F::One, O::Add),
            subtract && equation_separate && func_separate,
        ),
    ];
    for (mode, supported) in modes {
        assert_eq!(r.set_draw_blend_mode(mode).is_ok(), supported, "{mode:?}");
    }

    // dst - src, without separate alpha
    if subtract {
        let mode = BlendMode::compose_custom(
            F::One,
            F::One,
            O::RevSubtract,
            F::One,
            F::One,
            O::RevSubtract,
        );
        r.set_draw_blend_mode(BlendMode::NONE).unwrap();
        r.set_draw_color(100, 150, 200, 255);
        r.clear().unwrap();
        r.set_draw_blend_mode(mode).unwrap();
        r.set_draw_color(60, 60, 60, 0);
        r.render_fill_rect(Some(&FRect::new(10.0, 10.0, 30.0, 20.0)))
            .unwrap();
        let s = r.read_pixels(None).unwrap();
        assert_eq!(s.read_pixel(20, 20).unwrap(), Color::new(40, 90, 140, 255));
        assert_eq!(s.read_pixel(5, 5).unwrap(), Color::new(100, 150, 200, 255));
    }
    // (MIN and MAX aren't drawn on ANGLE: its glBlendEquationOES() takes
    // only GL_OES_blend_subtract's equations, its GL_EXT_blend_minmax being
    // OpenGL ES 2's, and leaves the equation as it was)
}

#[test]
#[cfg_attr(
    target_os = "emscripten",
    ignore = "no EGL on Emscripten's offscreen driver"
)]
fn gles_matches_software_for_shapes() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = gles_renderer(W, H) else {
        return;
    };

    compare("clear", &mut r, 0, false, |r| {
        r.set_draw_color(12, 34, 56, 255);
        r.clear().unwrap();
    });

    compare("opaque shapes", &mut r, 0, false, |r| {
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
    });

    compare("viewport and clip", &mut r, 0, false, |r| {
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.set_viewport(Some(&Rect::new(10, 6, 40, 30))).unwrap();
        r.set_clip_rect(Some(&Rect::new(5, 5, 20, 12))).unwrap();
        r.set_draw_color(250, 250, 0, 255);
        r.render_fill_rect(None).unwrap();
        r.set_clip_rect(None).unwrap();
        r.set_draw_color(0, 250, 250, 255);
        r.render_fill_rect(Some(&FRect::new(30.0, 20.0, 20.0, 20.0)))
            .unwrap();
        // (a clear clears past the clip rectangle, which comes back)
        r.set_clip_rect(Some(&Rect::new(0, 0, 4, 4))).unwrap();
        r.set_draw_color(9, 9, 9, 255);
        r.render_point(2.0, 2.0).unwrap();
        r.set_clip_rect(None).unwrap();
        r.set_viewport(None).unwrap();
    });

    let func_separate = has_extension(&mut r, "GL_OES_blend_func_separate");
    for (mode, tol) in [
        (BlendMode::BLEND, 2),
        (BlendMode::BLEND_PREMULTIPLIED, 2),
        (BlendMode::ADD, 2),
        (BlendMode::ADD_PREMULTIPLIED, 2),
        (BlendMode::MOD, 2),
        (BlendMode::MUL, 4),
    ] {
        // (GL blends in floats, the software renderer in integers; its MUL
        // fills drift furthest. Without GL_OES_blend_func_separate the
        // alpha channel blends with the color factors: only the colors
        // are compared then)
        compare(
            &format!("blended fill {mode:?}"),
            &mut r,
            tol,
            !func_separate,
            |r| {
                r.set_draw_blend_mode(BlendMode::NONE).unwrap();
                r.set_draw_color(40, 80, 120, 200);
                r.clear().unwrap();
                r.set_draw_blend_mode(mode).unwrap();
                r.set_draw_color(220, 60, 30, 128);
                r.render_fill_rect(Some(&FRect::new(6.0, 6.0, 40.0, 30.0)))
                    .unwrap();
                r.set_draw_color(20, 200, 90, 77);
                r.render_fill_rect(Some(&FRect::new(20.0, 16.0, 40.0, 30.0)))
                    .unwrap();
                r.render_line(0.0, 44.0, 63.0, 44.0).unwrap();
                r.set_draw_blend_mode(BlendMode::NONE).unwrap();
            },
        );
    }

    // Untextured geometry with per-vertex colors (the same color at each
    // corner, so that the interpolation is exact), then lines of the
    // draw color (glColor4f(), which the color array doesn't change).
    compare("geometry", &mut r, 0, false, |r| {
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        let c = FColor::new(1.0, 0.5, 0.25, 1.0);
        let v = |x: f32, y: f32| Vertex {
            position: FPoint { x, y },
            color: c,
            tex_coord: FPoint::default(),
        };
        r.render_geometry(
            None,
            &[v(8.0, 8.0), v(40.0, 8.0), v(40.0, 40.0), v(8.0, 40.0)],
            Some(&[0, 1, 2, 0, 2, 3]),
        )
        .unwrap();
        r.set_draw_color(0, 255, 0, 255);
        r.render_line(2.0, 44.0, 60.0, 44.0).unwrap();
    });
}

#[test]
#[cfg_attr(
    target_os = "emscripten",
    ignore = "no EGL on Emscripten's offscreen driver"
)]
fn gles_matches_software_for_textures() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = gles_renderer(W, H) else {
        return;
    };
    let mut sw = software_renderer(W, H);
    let func_separate = has_extension(&mut r, "GL_OES_blend_func_separate");

    // (16x12: the GL texture is 16x16, the coordinates scaled to its top
    // 12 rows)
    for format in [
        PixelFormat::ABGR8888, // (RGBA32: the renderer's own)
        PixelFormat::ARGB8888,
        PixelFormat::XRGB8888,
        PixelFormat::RGB565,
    ] {
        let tg = pattern_texture(&mut r, format, 16, 12);
        let ts = pattern_texture(&mut sw, format, 16, 12);
        for mode in [
            BlendMode::NONE,
            BlendMode::BLEND,
            BlendMode::ADD,
            BlendMode::MOD,
            BlendMode::MUL,
        ] {
            let scene = |r: &mut Renderer, t: Texture| {
                r.set_draw_color(30, 60, 90, 255);
                r.clear().unwrap();
                r.set_texture_blend_mode(t, mode).unwrap();
                r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
                r.set_texture_color_mod(t, 255, 255, 255).unwrap();
                r.set_texture_alpha_mod(t, 255).unwrap();
                r.render_texture(t, None, Some(&FRect::new(2.0, 3.0, 16.0, 12.0)))
                    .unwrap();
                // a part, scaled up 2x
                r.render_texture(
                    t,
                    Some(&FRect::new(4.0, 2.0, 8.0, 6.0)),
                    Some(&FRect::new(24.0, 4.0, 16.0, 12.0)),
                )
                .unwrap();
                // with color and alpha modulation
                r.set_texture_color_mod(t, 255, 128, 64).unwrap();
                r.set_texture_alpha_mod(t, 160).unwrap();
                r.render_texture(t, None, Some(&FRect::new(44.0, 30.0, 16.0, 12.0)))
                    .unwrap();
                // flipped (the software renderer's rotated copies with
                // BlendMode::NONE multiply the colors by alpha, as upstream's
                // do)
                if mode == BlendMode::NONE {
                    return;
                }
                r.set_texture_color_mod(t, 255, 255, 255).unwrap();
                r.set_texture_alpha_mod(t, 255).unwrap();
                r.render_texture_rotated(
                    t,
                    None,
                    Some(&FRect::new(4.0, 30.0, 16.0, 12.0)),
                    0.0,
                    None,
                    FlipMode::Horizontal,
                )
                .unwrap();
            };
            scene(&mut r, tg);
            scene(&mut sw, ts);
            let what = format!("{} texture, {mode:?}", format.name());
            // (modulated colors round differently: GL multiplies floats)
            let tol = if mode == BlendMode::NONE { 1 } else { 3 };
            assert_close_with(
                &what,
                &r.read_pixels(None).unwrap(),
                &sw.read_pixels(None).unwrap(),
                tol,
                mode != BlendMode::NONE && !func_separate,
            );
        }
        r.destroy_texture(tg);
        sw.destroy_texture(ts);
    }

    // A palettized texture (converted by the front end)
    let mut palette = Palette::new(256).unwrap();
    let colors: Vec<Color> = (0..256)
        .map(|i| Color::new(i as u8, 255 - i as u8, (i * 7) as u8, 255))
        .collect();
    palette.set_colors(0, &colors).unwrap();
    let palette = share_palette(palette);
    let indices: Vec<u8> = (0..16 * 12).map(|i| (i * 5) as u8).collect();
    let make = |r: &mut Renderer| {
        let t = r
            .create_texture(PixelFormat::INDEX8, TextureAccess::Static, 16, 12)
            .unwrap();
        r.set_texture_palette(t, Some(palette.clone())).unwrap();
        r.update_texture(t, None, &indices, 16).unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.render_texture(t, None, Some(&FRect::new(5.0, 5.0, 32.0, 24.0)))
            .unwrap();
        t
    };
    let tg = make(&mut r);
    let ts = make(&mut sw);
    assert_close(
        "palette texture",
        &r.read_pixels(None).unwrap(),
        &sw.read_pixels(None).unwrap(),
        0,
    );
    r.destroy_texture(tg);
    sw.destroy_texture(ts);

    // A YUV texture (converted by the front end)
    let (yw, yh) = (16usize, 12usize);
    let mut planes = vec![0u8; yw * yh + 2 * (yw / 2) * (yh / 2)];
    for (i, p) in planes.iter_mut().enumerate() {
        *p = if i < yw * yh {
            (60 + (i % yw) * 8) as u8
        } else {
            (110 + i % 30) as u8
        };
    }
    let make = |r: &mut Renderer| {
        let t = r
            .create_texture(PixelFormat::IYUV, TextureAccess::Static, 16, 12)
            .unwrap();
        r.update_texture(t, None, &planes, 16).unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.render_texture(t, None, Some(&FRect::new(5.0, 5.0, 32.0, 24.0)))
            .unwrap();
        t
    };
    let tg = make(&mut r);
    let ts = make(&mut sw);
    assert_close(
        "IYUV texture",
        &r.read_pixels(None).unwrap(),
        &sw.read_pixels(None).unwrap(),
        0,
    );
    r.destroy_texture(tg);
    sw.destroy_texture(ts);

    // A streaming texture, written through a lock and updated in part
    let make = |r: &mut Renderer| {
        let t = r
            .create_texture(PixelFormat::ABGR8888, TextureAccess::Streaming, 10, 10)
            .unwrap();
        {
            let mut lock = r.lock_texture(t, None).unwrap();
            lock.pixels().fill(0);
        }
        {
            let mut lock = r.lock_texture(t, Some(&Rect::new(2, 2, 6, 6))).unwrap();
            let pitch = lock.pitch() as usize;
            let pixels = lock.pixels();
            for y in 0..6 {
                for x in 0..6 {
                    pixels[y * pitch + x * 4..][..4].copy_from_slice(&[
                        40 * x as u8,
                        40 * y as u8,
                        200,
                        255,
                    ]);
                }
            }
        }
        // (an update of a part, with rows further apart than its width)
        let part: Vec<u8> = (0..3 * 12).map(|i| (i * 7) as u8).collect();
        r.update_texture(t, Some(&Rect::new(6, 1, 2, 3)), &part, 12)
            .unwrap();
        r.set_texture_blend_mode(t, BlendMode::NONE).unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.render_texture(t, None, Some(&FRect::new(10.0, 10.0, 20.0, 20.0)))
            .unwrap();
        t
    };
    let tg = make(&mut r);
    let ts = make(&mut sw);
    assert_close(
        "streaming texture",
        &r.read_pixels(None).unwrap(),
        &sw.read_pixels(None).unwrap(),
        0,
    );
    r.destroy_texture(tg);
    sw.destroy_texture(ts);

    // Linear scaling clamps at the edges (`FIXME (upstream)` in
    // `create_texture`: upstream leaves GL's default address modes,
    // GL_REPEAT, which blend the edges with the opposite ones)
    let t = pattern_texture(&mut r, PixelFormat::ABGR8888, 16, 16);
    r.set_texture_scale_mode(t, ScaleMode::Linear).unwrap();
    r.set_texture_blend_mode(t, BlendMode::NONE).unwrap();
    r.set_draw_color(0, 0, 0, 255);
    r.clear().unwrap();
    r.render_texture(t, None, Some(&FRect::new(0.0, 0.0, 48.0, 48.0)))
        .unwrap();
    let s = r.read_pixels(Some(&Rect::new(0, 0, 48, 48))).unwrap();
    let p = pattern(16, 16);
    for (x, y, texel) in [(0, 0, 0), (47, 0, 15), (0, 47, 15 * 16), (47, 47, 255)] {
        let c = s.read_pixel(x, y).unwrap();
        let e = &p[texel * 4..][..3];
        assert!(
            [c.r, c.g, c.b]
                .iter()
                .zip(e)
                .all(|(a, b)| a.abs_diff(*b) <= 2),
            "linear scaling at ({x}, {y}): {c:?}, the texel {e:?}"
        );
    }
    r.destroy_texture(t);

    // Wrapping texture coordinates (of a power of two texture: the
    // coordinates of the others are scaled into the GL texture, whose
    // padding the wrap would reach, as upstream's)
    let scene = |r: &mut Renderer, t: Texture| {
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.set_texture_blend_mode(t, BlendMode::NONE).unwrap();
        r.set_texture_address_mode(TextureAddressMode::Wrap, TextureAddressMode::Wrap);
        let white = FColor::new(1.0, 1.0, 1.0, 1.0);
        let v = |x: f32, y: f32, u: f32, w: f32| Vertex {
            position: FPoint { x, y },
            color: white,
            tex_coord: FPoint { x: u, y: w },
        };
        r.render_geometry(
            Some(t),
            &[
                v(0.0, 0.0, 0.0, 0.0),
                v(32.0, 0.0, 2.0, 0.0),
                v(32.0, 32.0, 2.0, 2.0),
                v(0.0, 32.0, 0.0, 2.0),
            ],
            Some(&[0, 1, 2, 0, 2, 3]),
        )
        .unwrap();
        r.set_texture_address_mode(TextureAddressMode::Auto, TextureAddressMode::Auto);
    };
    let tg = pattern_texture(&mut r, PixelFormat::ABGR8888, 16, 16);
    let ts = pattern_texture(&mut sw, PixelFormat::ABGR8888, 16, 16);
    scene(&mut r, tg);
    scene(&mut sw, ts);
    assert_close(
        "wrapping geometry",
        &r.read_pixels(None).unwrap(),
        &sw.read_pixels(None).unwrap(),
        0,
    );
    r.destroy_texture(tg);
    sw.destroy_texture(ts);
}

#[test]
#[cfg_attr(
    target_os = "emscripten",
    ignore = "no EGL on Emscripten's offscreen driver"
)]
fn gles_matches_software_for_targets() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = gles_renderer(W, H) else {
        return;
    };
    let mut sw = software_renderer(W, H);
    let func_separate = has_extension(&mut r, "GL_OES_blend_func_separate");

    // (ARGB8888 targets are RGBA32 ones, the renderer's only format; 32x24
    // is a 32x32 GL texture)
    for format in [PixelFormat::ABGR8888, PixelFormat::ARGB8888] {
        let scene = |r: &mut Renderer, src: Texture| -> (Surface<'static>, Surface<'static>) {
            let target = r
                .create_texture(format, TextureAccess::Target, 32, 24)
                .unwrap();
            r.set_texture_scale_mode(target, ScaleMode::Nearest)
                .unwrap();
            r.set_render_target(Some(target)).unwrap();
            r.set_draw_color(250, 40, 10, 255);
            r.clear().unwrap();
            r.set_draw_color(10, 40, 250, 255);
            r.render_fill_rect(Some(&FRect::new(2.0, 3.0, 10.0, 6.0)))
                .unwrap();
            r.render_point(30.0, 1.0).unwrap();
            r.set_viewport(Some(&Rect::new(16, 0, 16, 10))).unwrap();
            r.set_clip_rect(Some(&Rect::new(2, 2, 6, 6))).unwrap();
            r.render_fill_rect(None).unwrap();
            r.set_clip_rect(None).unwrap();
            r.set_viewport(None).unwrap();
            r.set_texture_blend_mode(src, BlendMode::NONE).unwrap();
            r.set_texture_scale_mode(src, ScaleMode::Nearest).unwrap();
            r.render_texture(src, None, Some(&FRect::new(14.0, 10.0, 16.0, 12.0)))
                .unwrap();
            let read = r.read_pixels(Some(&Rect::new(1, 2, 30, 20))).unwrap();
            r.set_render_target(None).unwrap();
            r.set_draw_color(0, 0, 0, 255);
            r.clear().unwrap();
            r.render_texture(target, None, Some(&FRect::new(4.0, 4.0, 32.0, 24.0)))
                .unwrap();
            r.render_texture(target, None, Some(&FRect::new(36.0, 20.0, 16.0, 12.0)))
                .unwrap();
            let out = r.read_pixels(None).unwrap();
            r.destroy_texture(target);
            (read, out)
        };
        let tg = pattern_texture(&mut r, PixelFormat::ABGR8888, 16, 12);
        let ts = pattern_texture(&mut sw, PixelFormat::ABGR8888, 16, 12);
        let (gl_read, gl_out) = scene(&mut r, tg);
        let (sw_read, sw_out) = scene(&mut sw, ts);
        let what = format!("{} target", format.name());
        assert_close(&format!("{what} read"), &gl_read, &sw_read, 0);
        // (the target is drawn blended, rounding differently, and without
        // GL_OES_blend_func_separate its alpha channel with the color
        // factors)
        assert_close_with(&what, &gl_out, &sw_out, 2, !func_separate);
        r.destroy_texture(tg);
        sw.destroy_texture(ts);
    }
}

#[test]
#[cfg_attr(
    target_os = "emscripten",
    ignore = "no EGL on Emscripten's offscreen driver"
)]
fn gles_line_methods() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    // GL lines (SDL_RENDER_LINE_METHOD 2) hit the same pixels as the
    // software renderer's for straight lines, except maybe the last pixel
    // of a line: the end is pushed out a quarter pixel to "provoke the
    // diamond-exit rule", which isn't enough for a rasterizer that follows
    // the rule exactly.
    hints::set(hints::RENDER_LINE_METHOD, "2").unwrap();
    let created = gles_renderer(W, H);
    hints::reset(hints::RENDER_LINE_METHOD);
    let Some((_window, mut r)) = created else {
        return;
    };
    let scene = |r: &mut Renderer| {
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.set_draw_color(255, 128, 0, 255);
        r.render_line(2.0, 4.0, 60.0, 4.0).unwrap();
        r.render_line(7.0, 10.0, 7.0, 40.0).unwrap();
        r.render_lines(&[
            FPoint { x: 20.0, y: 20.0 },
            FPoint { x: 40.0, y: 20.0 },
            FPoint { x: 40.0, y: 30.0 },
        ])
        .unwrap();
        r.read_pixels(None).unwrap()
    };
    let gl = scene(&mut r);
    let sw = scene(&mut software_renderer(W, H));
    for ((x, y), pa, pb) in differences(&gl, &sw, 0, false) {
        assert!(
            [(60, 4), (7, 40), (40, 30)].contains(&(x, y)),
            "GL lines differ at ({x}, {y}): gl {pa:?} sw {pb:?}"
        );
    }
}

/// The renderer makes its own context current to draw (and to make and
/// update textures) when the application's is
/// (`GLES_ActivateRenderer()`), and the application's context can be made
/// current again between frames.
#[test]
#[cfg_attr(
    target_os = "emscripten",
    ignore = "no EGL on Emscripten's offscreen driver"
)]
fn gles_activates_its_context() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((window, mut r)) = gles_renderer(W, H) else {
        return;
    };
    let scene = |r: &mut Renderer| {
        r.set_draw_color(20, 40, 60, 255);
        r.clear().unwrap();
        let t = pattern_texture(r, PixelFormat::ABGR8888, 8, 8);
        r.set_texture_blend_mode(t, BlendMode::NONE).unwrap();
        r.render_texture(t, None, Some(&FRect::new(4.0, 4.0, 8.0, 8.0)))
            .unwrap();
        r.set_draw_color(200, 0, 0, 255);
        r.render_fill_rect(Some(&FRect::new(30.0, 4.0, 8.0, 8.0)))
            .unwrap();
        r.destroy_texture(t);
    };
    scene(&mut r);
    let expected = r.read_pixels(None).unwrap();

    // (the application's own OpenGL ES 1.1 context on the window)
    let context = GlContext::new(&window).unwrap();
    assert!(context.is_current());
    scene(&mut r);
    assert!(!context.is_current());
    let again = r.read_pixels(None).unwrap();
    assert_close("with another context current", &again, &expected, 0);
    context.make_current(Some(&window)).unwrap();
    assert!(context.is_current());
    r.present().unwrap();
    assert!(!context.is_current());
    drop(context);
    scene(&mut r);
    assert_close(
        "after the other context",
        &r.read_pixels(None).unwrap(),
        &expected,
        0,
    );
}

/// Note (upstream): the renderer takes any output colorspace (it doesn't
/// check what `SDL_SetupRendererColorspace()` picked), and draws as if it
/// were sRGB.
#[test]
#[cfg_attr(
    target_os = "emscripten",
    ignore = "no EGL on Emscripten's offscreen driver"
)]
fn gles_takes_a_linear_output_colorspace() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let info = RendererCreateInfo {
        output_colorspace: Some(Colorspace::SRGB_LINEAR),
        ..Default::default()
    };
    let Some((_window, r)) = gles_renderer_with(W, H, &info) else {
        return;
    };
    assert_eq!(r.name(), GLES_RENDERER);
    assert_eq!(
        r.properties()
            .get_number(PROP_RENDERER_OUTPUT_COLORSPACE_NUMBER),
        Some(Colorspace::SRGB_LINEAR.0 as i64)
    );
    // (the software renderer refuses it)
    assert!(
        Renderer::software_with(Surface::new(8, 8, PixelFormat::RGBA32).unwrap(), &info).is_err()
    );
}
