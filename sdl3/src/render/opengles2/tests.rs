// Tests of the OpenGL ES 2.0 renderer: scenes drawn through a real OpenGL
// ES context (the offscreen driver's EGL; Mesa's llvmpipe on Linux CI) and
// through the software renderer, compared pixel by pixel. Without an
// OpenGL ES library (Windows CI, Wine) the GL tests skip (capability `egl`).

use crate::events::window::WindowFlags;
use crate::hints;
use crate::init::{self, InitFlags};
use crate::render::{
    Renderer, Texture, TextureAccess, Vertex, PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER,
    PROP_RENDERER_TEXTURE_WRAPPING_BOOLEAN, PROP_TEXTURE_OPENGLES2_TEXTURE_NUMBER,
    PROP_TEXTURE_OPENGLES2_TEXTURE_TARGET_NUMBER, PROP_TEXTURE_OPENGLES2_TEXTURE_UV_NUMBER,
    PROP_TEXTURE_OPENGLES2_TEXTURE_U_NUMBER, PROP_TEXTURE_OPENGLES2_TEXTURE_V_NUMBER,
    SOFTWARE_RENDERER,
};
use crate::video::pixels::{Color, FColor, Palette, PixelFormat};
use crate::video::rect::{FPoint, FRect, Rect};
use crate::video::surface::{share_palette, ScaleMode, Surface};
use crate::video::{BlendMode, FlipMode, Window};

use super::GLES2_RENDERER;

const W: i32 = 64;
const H: i32 = 48;

/// Video up on the offscreen driver; quit when dropped.
struct Video;

impl Video {
    fn init() -> Video {
        init::quit();
        hints::set(hints::VIDEO_DRIVER, "offscreen").unwrap();
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

/// An offscreen window with an OpenGL ES 2.0 renderer, or `None` (with the
/// skip reported) when there is no OpenGL ES.
fn gles2_renderer(w: i32, h: i32) -> Option<(Window, Renderer)> {
    let window = Window::create("gles2", w, h, WindowFlags::default()).unwrap();
    match Renderer::for_window(&window, Some(GLES2_RENDERER)) {
        Ok(r) => Some((window, r)),
        Err(e) => {
            crate::test_support::skip(
                "egl",
                format_args!("no OpenGL ES 2 renderer ({})", e.message()),
            );
            window.destroy();
            None
        }
    }
}

/// A software renderer drawing into an RGBA32 surface (the format the
/// GLES2 renderer reads the window back in).
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

/// The pixels where two images differ by more than `tol` in a channel.
fn differences(gl: &Surface<'_>, sw: &Surface<'_>, tol: u8) -> Vec<Difference> {
    let (a, b) = (rows(gl), rows(sw));
    let bpp = gl.format().bytes_per_pixel() as usize;
    let w = gl.width() as usize;
    // (the padding byte of XRGB8888 and XBGR8888 isn't compared)
    let padding = matches!(gl.format(), PixelFormat::XRGB8888 | PixelFormat::XBGR8888).then_some(3);
    a.chunks(bpp)
        .zip(b.chunks(bpp))
        .enumerate()
        .filter(|(_, (pa, pb))| {
            pa.iter()
                .zip(*pb)
                .enumerate()
                .any(|(c, (x, y))| Some(c) != padding && x.abs_diff(*y) > tol)
        })
        .map(|(i, (pa, pb))| (((i % w) as i32, (i / w) as i32), pa.to_vec(), pb.to_vec()))
        .collect()
}

/// Compare two images, each channel within `tol` (the software image is
/// converted to the GL image's format first).
fn assert_close(what: &str, gl: &Surface<'_>, sw: &Surface<'_>, tol: u8) {
    let converted;
    let sw = if sw.format() != gl.format() {
        converted = sw.convert(gl.format()).unwrap();
        &converted
    } else {
        sw
    };
    assert_eq!(
        (gl.width(), gl.height()),
        (sw.width(), sw.height()),
        "{what}: sizes"
    );
    let bad = differences(gl, sw, tol);
    if let Some(((x, y), pa, pb)) = bad.first() {
        panic!(
            "{what}: {} pixels differ by more than {tol}; first at ({x}, {y}): gl {pa:?} sw {pb:?}",
            bad.len()
        );
    }
}

/// Draw `scene` with the GLES2 renderer and the software renderer, and
/// compare the results (`tol` per channel).
fn compare(what: &str, r: &mut Renderer, tol: u8, scene: impl Fn(&mut Renderer)) {
    scene(r);
    let gl = r.read_pixels(None).unwrap();
    let mut sw = software_renderer(W, H);
    scene(&mut sw);
    let sw = sw.read_pixels(None).unwrap();
    assert_close(what, &gl, &sw, tol);
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

#[test]
#[cfg(not(windows))] // (direct3d11 comes first there: see its tests)
fn driver_list_follows_upstreams_order() {
    use crate::render::{num_render_drivers, render_driver};
    assert_eq!(num_render_drivers(), 4);
    assert_eq!(render_driver(0).unwrap(), "opengl");
    assert_eq!(render_driver(1).unwrap(), "opengles2");
    assert_eq!(render_driver(2).unwrap(), "vulkan");
    assert_eq!(render_driver(3).unwrap(), SOFTWARE_RENDERER);
    assert!(render_driver(4).is_err());
}

#[test]
fn gles2_renderer_basics() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((window, mut r)) = gles2_renderer(W, H) else {
        return;
    };
    assert_eq!(r.name(), "opengles2");
    assert!(window.flags().unwrap().contains(WindowFlags::OPENGL));
    assert_eq!(r.output_size().unwrap(), (W, H));
    let props = r.properties();
    assert!(
        props
            .get_number(PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER)
            .unwrap()
            >= 64
    );
    assert!(props
        .get_bool(PROP_RENDERER_TEXTURE_WRAPPING_BOOLEAN)
        .is_some());
    assert_eq!(
        &r.texture_formats()[..10],
        &[
            PixelFormat::BGRA32,
            PixelFormat::RGBA32,
            PixelFormat::BGRX32,
            PixelFormat::RGBX32,
            PixelFormat::INDEX8,
            PixelFormat::YV12,
            PixelFormat::IYUV,
            PixelFormat::I444,
            PixelFormat::NV12,
            PixelFormat::NV21,
        ]
    );

    // The GL texture names are published as properties.
    let t = r
        .create_texture(PixelFormat::ARGB8888, TextureAccess::Static, 8, 8)
        .unwrap();
    let tp = r.texture_properties(t).unwrap();
    assert!(
        tp.get_number(PROP_TEXTURE_OPENGLES2_TEXTURE_NUMBER)
            .unwrap()
            > 0
    );
    assert_eq!(
        tp.get_number(PROP_TEXTURE_OPENGLES2_TEXTURE_TARGET_NUMBER),
        Some(0x0DE1) // GL_TEXTURE_2D
    );
    let yuv = r
        .create_texture(PixelFormat::IYUV, TextureAccess::Static, 8, 8)
        .unwrap();
    let yp = r.texture_properties(yuv).unwrap();
    assert!(
        yp.get_number(PROP_TEXTURE_OPENGLES2_TEXTURE_U_NUMBER)
            .unwrap()
            > 0
    );
    assert!(
        yp.get_number(PROP_TEXTURE_OPENGLES2_TEXTURE_V_NUMBER)
            .unwrap()
            > 0
    );
    let nv = r
        .create_texture(PixelFormat::NV12, TextureAccess::Static, 8, 8)
        .unwrap();
    let np = r.texture_properties(nv).unwrap();
    assert!(
        np.get_number(PROP_TEXTURE_OPENGLES2_TEXTURE_UV_NUMBER)
            .unwrap()
            > 0
    );
    // (an unsupported YUV colorspace is refused)
    assert!(r
        .create_texture_with(&crate::render::TextureCreateInfo {
            format: PixelFormat::NV12,
            width: 8,
            height: 8,
            colorspace: Some(crate::video::pixels::Colorspace::SRGB),
            ..Default::default()
        })
        .is_err());

    // Custom blend modes that GLES2 can do are supported
    let custom = BlendMode::compose_custom(
        crate::video::blendmode::BlendFactor::One,
        crate::video::blendmode::BlendFactor::One,
        crate::video::blendmode::BlendOperation::RevSubtract,
        crate::video::blendmode::BlendFactor::Zero,
        crate::video::blendmode::BlendFactor::One,
        crate::video::blendmode::BlendOperation::Add,
    );
    r.set_draw_blend_mode(custom).unwrap();
    r.set_draw_blend_mode(BlendMode::NONE).unwrap();

    r.set_draw_color(1, 2, 3, 255);
    r.clear().unwrap();
    r.present().unwrap();
    r.set_vsync(0).unwrap();
    assert_eq!(r.vsync(), 0);

    // The default driver on a GL capable video driver is a GL one (the
    // OpenGL renderer comes first, where it can be made).
    drop(r);
    window.destroy();
    let w2 = Window::create("gles2 default", 16, 16, WindowFlags::default()).unwrap();
    let r2 = Renderer::for_window(&w2, None).unwrap();
    assert!(matches!(r2.name(), "opengl" | "opengles2"), "{}", r2.name());
    // The window can go first.
    w2.destroy();
    drop(r2);
}

#[test]
fn gles2_matches_software_for_shapes() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = gles2_renderer(W, H) else {
        return;
    };

    compare("clear", &mut r, 0, |r| {
        r.set_draw_color(12, 34, 56, 255);
        r.clear().unwrap();
    });

    compare("opaque shapes", &mut r, 0, |r| {
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
        r.render_line(5.0, 20.0, 25.0, 40.0).unwrap();
        r.render_rect(Some(&FRect::new(8.0, 24.0, 14.0, 10.0)))
            .unwrap();
    });

    compare("viewport and clip", &mut r, 0, |r| {
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
        r.set_viewport(None).unwrap();
    });

    for (mode, tol) in [
        (BlendMode::BLEND, 2),
        (BlendMode::BLEND_PREMULTIPLIED, 2),
        (BlendMode::ADD, 2),
        (BlendMode::ADD_PREMULTIPLIED, 2),
        (BlendMode::MOD, 2),
        (BlendMode::MUL, 4),
    ] {
        // (GL blends in floats, the software renderer in integers; its MUL
        // fills drift furthest)
        compare(&format!("blended fill {mode:?}"), &mut r, tol, |r| {
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
            r.set_draw_blend_mode(BlendMode::NONE).unwrap();
        });
    }

    // Untextured geometry with per-vertex colors (the same color at each
    // corner, so that the interpolation is exact).
    compare("geometry", &mut r, 0, |r| {
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
    });
}

#[test]
fn gles2_matches_software_for_textures() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = gles2_renderer(W, H) else {
        return;
    };
    let mut sw = software_renderer(W, H);

    for format in [
        PixelFormat::ARGB8888,
        PixelFormat::ABGR8888,
        PixelFormat::XRGB8888,
        PixelFormat::XBGR8888,
        PixelFormat::RGB565, // (through a native texture)
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
                // do: SW_RenderCopyEx() adds the colors from a surface that
                // still has the alpha channel)
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
            assert_close(
                &what,
                &r.read_pixels(None).unwrap(),
                &sw.read_pixels(None).unwrap(),
                tol,
            );
        }
        r.destroy_texture(tg);
        sw.destroy_texture(ts);
    }

    // A palettized texture
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

    // A streaming texture, written through a lock
    let make = |r: &mut Renderer| {
        let t = r
            .create_texture(PixelFormat::ABGR8888, TextureAccess::Streaming, 10, 10)
            .unwrap();
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
    let gl_streaming = r.read_pixels(Some(&Rect::new(10, 10, 20, 20))).unwrap();
    let sw_streaming = sw.read_pixels(Some(&Rect::new(10, 10, 20, 20))).unwrap();
    // (the unlocked parts of a new streaming texture are zero in GLES2,
    // undefined in the software renderer: compare the locked part)
    let crop = |s: &Surface<'_>| {
        let mut c = Surface::new(12, 12, s.format()).unwrap();
        let mut src = s.duplicate().unwrap();
        src.set_blend_mode(BlendMode::NONE).unwrap();
        src.blit(Some(&Rect::new(4, 4, 12, 12)), &mut c, None)
            .unwrap();
        c
    };
    assert_close(
        "streaming texture",
        &crop(&gl_streaming),
        &crop(&sw_streaming),
        0,
    );
    r.destroy_texture(tg);
    sw.destroy_texture(ts);
}

#[test]
fn gles2_matches_software_for_targets() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = gles2_renderer(W, H) else {
        return;
    };
    let mut sw = software_renderer(W, H);

    for format in [
        PixelFormat::ABGR8888,
        PixelFormat::ARGB8888,
        PixelFormat::XRGB8888,
        PixelFormat::XBGR8888,
    ] {
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
        assert_eq!(gl_read.format(), format, "{what}: read format");
        assert_close(&format!("{what} read"), &gl_read, &sw_read, 0);
        // (the target is drawn blended, rounding differently)
        assert_close(&what, &gl_out, &sw_out, 2);
        r.destroy_texture(tg);
        sw.destroy_texture(ts);
    }
}

/// Planes of a test YUV image. The colors stay inside the RGB gamut: the
/// software conversion (`yuv_rgb_std.c`) only clamps inputs in
/// [-128, 384), as upstream's does.
fn yuv_planes(w: usize, h: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let y: Vec<u8> = (0..w * h)
        .map(|i| (60 + (i % w) * 100 / w + (i / w) * 20 / h) as u8)
        .collect();
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let u: Vec<u8> = (0..cw * ch)
        .map(|i| (110 + (i % cw) * 36 / cw) as u8)
        .collect();
    let v: Vec<u8> = (0..cw * ch)
        .map(|i| (146 - (i / cw) * 36 / ch) as u8)
        .collect();
    (y, u, v)
}

#[test]
fn gles2_matches_software_for_yuv() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = gles2_renderer(W, H) else {
        return;
    };
    let mut sw = software_renderer(W, H);
    let (w, h) = (16usize, 12usize);
    let (y, u, v) = yuv_planes(w, h);
    let uv: Vec<u8> = u.iter().zip(&v).flat_map(|(a, b)| [*a, *b]).collect();
    let vu: Vec<u8> = u.iter().zip(&v).flat_map(|(a, b)| [*b, *a]).collect();

    for format in [
        PixelFormat::IYUV,
        PixelFormat::YV12,
        PixelFormat::NV12,
        PixelFormat::NV21,
    ] {
        for access in [TextureAccess::Static, TextureAccess::Streaming] {
            let scene = |r: &mut Renderer| {
                let t = r
                    .create_texture(format, access, w as i32, h as i32)
                    .unwrap();
                r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
                match format {
                    PixelFormat::NV12 => r
                        .update_nv_texture(t, None, &y, w as i32, &uv, w as i32)
                        .unwrap(),
                    PixelFormat::NV21 => r
                        .update_nv_texture(t, None, &y, w as i32, &vu, w as i32)
                        .unwrap(),
                    _ => r
                        .update_yuv_texture(
                            t,
                            None,
                            &y,
                            w as i32,
                            &u,
                            w as i32 / 2,
                            &v,
                            w as i32 / 2,
                        )
                        .unwrap(),
                }
                r.set_draw_color(0, 0, 0, 255);
                r.clear().unwrap();
                r.render_texture(t, None, Some(&FRect::new(3.0, 3.0, 32.0, 24.0)))
                    .unwrap();
                r.destroy_texture(t);
                r.read_pixels(None).unwrap()
            };
            let what = format!("{} {access:?} texture", format.name());
            // (the shaders convert in floats, the software path in fixed point)
            assert_close(&what, &scene(&mut r), &scene(&mut sw), 3);
        }
    }

    // A whole-texture update with all the planes in one buffer.
    let mut all = y.clone();
    all.extend_from_slice(&u);
    all.extend_from_slice(&v);
    let scene = |r: &mut Renderer| {
        let t = r
            .create_texture(PixelFormat::IYUV, TextureAccess::Static, w as i32, h as i32)
            .unwrap();
        r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
        r.update_texture(t, None, &all, w as i32).unwrap();
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.render_texture(t, None, Some(&FRect::new(3.0, 3.0, 32.0, 24.0)))
            .unwrap();
        r.destroy_texture(t);
        r.read_pixels(None).unwrap()
    };
    assert_close("IYUV update", &scene(&mut r), &scene(&mut sw), 3);
    // (a buffer without the chroma planes is refused, not read past its end)
    let t = r
        .create_texture(PixelFormat::IYUV, TextureAccess::Static, w as i32, h as i32)
        .unwrap();
    assert!(r.update_texture(t, None, &y, w as i32).is_err());
}

/// More programs than the cache holds, used in turn (upstream's linked list
/// cache gets this wrong).
#[test]
fn gles2_program_cache_eviction() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    let Some((_window, mut r)) = gles2_renderer(W, H) else {
        return;
    };
    let mut sw = software_renderer(W, H);
    let formats = [
        PixelFormat::ARGB8888,
        PixelFormat::ABGR8888,
        PixelFormat::XRGB8888,
        PixelFormat::XBGR8888,
        PixelFormat::IYUV,
        PixelFormat::NV12,
        PixelFormat::NV21,
        PixelFormat::INDEX8,
    ];
    let make = |r: &mut Renderer| -> Vec<Texture> {
        let (y, u, v) = yuv_planes(8, 8);
        let uv: Vec<u8> = u.iter().zip(&v).flat_map(|(a, b)| [*a, *b]).collect();
        let mut palette = Palette::new(256).unwrap();
        palette
            .set_colors(0, &[Color::new(10, 200, 30, 255); 256])
            .unwrap();
        let palette = share_palette(palette);
        formats
            .iter()
            .map(|&f| {
                let t = match f {
                    PixelFormat::IYUV => {
                        let t = r.create_texture(f, TextureAccess::Static, 8, 8).unwrap();
                        r.update_yuv_texture(t, None, &y, 8, &u, 4, &v, 4).unwrap();
                        t
                    }
                    PixelFormat::NV12 | PixelFormat::NV21 => {
                        let t = r.create_texture(f, TextureAccess::Static, 8, 8).unwrap();
                        r.update_nv_texture(t, None, &y, 8, &uv, 8).unwrap();
                        t
                    }
                    PixelFormat::INDEX8 => {
                        let t = r.create_texture(f, TextureAccess::Static, 8, 8).unwrap();
                        r.set_texture_palette(t, Some(palette.clone())).unwrap();
                        r.update_texture(t, None, &[3; 64], 8).unwrap();
                        t
                    }
                    _ => pattern_texture(r, f, 8, 8),
                };
                r.set_texture_scale_mode(t, ScaleMode::Nearest).unwrap();
                r.set_texture_blend_mode(t, BlendMode::NONE).unwrap();
                t
            })
            .collect()
    };
    let tg = make(&mut r);
    let ts = make(&mut sw);
    let scene = |r: &mut Renderer, textures: &[Texture]| {
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        for round in 0..3 {
            for (i, &t) in textures.iter().enumerate() {
                let x = (i * 8) as f32;
                let y = (round * 12) as f32;
                r.render_texture(t, None, Some(&FRect::new(x, y, 8.0, 8.0)))
                    .unwrap();
                // and the solid shader, and a palette in linear mode
                r.set_draw_color(255, 255, 255, 255);
                r.render_point(x, y + 9.0).unwrap();
            }
            r.flush().unwrap();
        }
        r.read_pixels(None).unwrap()
    };
    assert_close(
        "many programs",
        &scene(&mut r, &tg),
        &scene(&mut sw, &ts),
        3,
    );

    // Linear palette sampling and pixel art use further programs.
    r.set_texture_scale_mode(tg[7], ScaleMode::Linear).unwrap();
    r.render_texture(tg[7], None, None).unwrap();
    r.set_texture_scale_mode(tg[0], ScaleMode::PixelArt)
        .unwrap();
    r.render_texture(tg[0], None, None).unwrap();
    assert_close(
        "many programs again",
        &scene(&mut r, &tg),
        &scene(&mut sw, &ts),
        3,
    );
}

#[test]
fn gles2_line_methods() {
    let _l = crate::test_support::test_lock();
    let _video = Video::init();
    // GL lines (SDL_RENDER_LINE_METHOD 2) hit the same pixels as the
    // software renderer's for straight lines, except maybe the last pixel
    // of a line: the end is pushed out a quarter pixel to "provoke the
    // diamond-exit rule", which isn't enough for a rasterizer that follows
    // the rule exactly (as llvmpipe does).
    hints::set(hints::RENDER_LINE_METHOD, "2").unwrap();
    let created = gles2_renderer(W, H);
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
    for ((x, y), pa, pb) in differences(&gl, &sw, 0) {
        assert!(
            [(60, 4), (7, 40), (40, 30)].contains(&(x, y)),
            "GL lines differ at ({x}, {y}): gl {pa:?} sw {pb:?}"
        );
    }
}

#[test]
fn gles2_skips_without_a_gl_driver() {
    let _l = crate::test_support::test_lock();
    // The dummy driver has no GL: the renderer can't be created, and the
    // window is put back the way it was.
    init::quit();
    hints::set(hints::VIDEO_DRIVER, "dummy").unwrap();
    init::init(InitFlags::VIDEO).unwrap();
    let window = Window::create("no gl", 16, 16, WindowFlags::default()).unwrap();
    assert!(Renderer::for_window(&window, Some(GLES2_RENDERER)).is_err());
    assert!(!window.flags().unwrap().contains(WindowFlags::OPENGL));
    let r = Renderer::for_window(&window, None).unwrap();
    assert_eq!(r.name(), SOFTWARE_RENDERER);
    drop(r);
    window.destroy();
    init::quit();
    hints::reset(hints::VIDEO_DRIVER);
}
