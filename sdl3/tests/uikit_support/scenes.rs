// The renderer scenes the UIKit renderer test programs draw (the C
// programs draw the same ones), and the trace they write.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Each scene is drawn by the renderer on the window and read back (the
//! top left `RW`x`RH` pixels, traced as their hash, to compare with
//! upstream's), then drawn by the software renderer and compared with it,
//! within the tolerances the other renderers' tests allow (what differed
//! is kept in [`MISMATCHES`], checked after the trace).

use std::sync::Mutex;

use sdl3::render::{Renderer, TextureAccess, TextureAddressMode, Vertex};
use sdl3::video::{
    self, BlendFactor, BlendMode, BlendOperation, Color, FColor, FPoint, FRect, PixelFormat, Rect,
    ScaleMode, Surface,
};

/// The program's trace.
pub static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// A line of the trace (printed too).
pub fn t(line: impl Into<String>) {
    let line = line.into();
    println!("trace: {line}");
    TRACE.lock().unwrap().push(line);
}

/// What differed from the software renderer (checked after the trace).
pub static MISMATCHES: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub const RW: i32 = 64;
pub const RH: i32 = 48;

/// The pixels of a surface as tightly packed RGBA32.
pub fn rgba(surface: &Surface<'_>) -> (i32, i32, Vec<u8>) {
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
pub fn hash_pixels(label: &str, image: &(i32, i32, Vec<u8>)) {
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
pub fn hash_read(label: &str, read: sdl3::Result<Surface<'static>>) -> Option<(i32, i32, Vec<u8>)> {
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
    renderer: &str,
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
            "{what}: {bad} pixels differ (first at {first:?}: {renderer} vs software)"
        ));
    }
}

pub type Scene<'a> = &'a dyn Fn(&mut Renderer) -> sdl3::Result<()>;

/// Draw a scene, read back the top left RWxRH pixels (traced as their
/// hash), present; and compare with the software renderer's pixels
/// (`None`: not compared).
pub fn scene(r: &mut Renderer, name: &str, compare_with: Option<(u8, usize)>, draw: Scene<'_>) {
    let label = format!("scene {name}");
    if let Err(e) = draw(r) {
        t(format!("{label}: error: {e}"));
    }
    let image = hash_read(&label, r.read_pixels(Some(&Rect::new(0, 0, RW, RH))));
    let _ = r.present();
    // (the software renderer can't be made on visionOS, whose renderers'
    // output colorspace is always linear sRGB, as upstream's: the hashes
    // are compared with upstream's alone there)
    let compare_with = compare_with.filter(|_| !cfg!(target_os = "visionos"));
    if let (Some(image), Some((tolerance, max_bad))) = (image, compare_with) {
        let mut sw =
            Renderer::software(Surface::new(RW, RH, PixelFormat::ARGB8888).unwrap()).unwrap();
        match draw(&mut sw) {
            Ok(()) => {
                let b = rgba(&sw.read_pixels(None).unwrap());
                compare(&label, r.name(), &image, &b, tolerance, max_bad);
            }
            Err(e) => MISMATCHES
                .lock()
                .unwrap()
                .push(format!("{label}: software: {e}")),
        }
    }
}

/// A 16x16 pattern with varying alpha.
pub fn pattern_color(x: i32, y: i32) -> Color {
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
pub fn pattern_surface(format: PixelFormat) -> Surface<'static> {
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

pub fn clear_and_fill(r: &mut Renderer) -> sdl3::Result<()> {
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

fn debug_text(r: &mut Renderer, text: &str) -> sdl3::Result<()> {
    r.set_draw_color(0, 0, 0, 255);
    r.clear()?;
    r.set_draw_color(255, 255, 255, 255);
    r.render_debug_text(2.0, 2.0, text)?;
    r.render_debug_text(2.0, 20.0, "SDL 3")
}

/// The renderer's scenes (the debug text one draws `text`).
pub fn scenes(r: &mut Renderer, text: &str) {
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
    scene(r, "debug text", Some((0, 0)), &|r| debug_text(r, text));

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

/// The custom blend mode's pixels: dst - src for the color (in sRGB; on
/// visionOS the renderers blend in linear sRGB, so there only upstream's
/// hash of the scene is compared).
pub fn check_custom_blend(r: &mut Renderer) {
    custom_blend(r).unwrap();
    let s = r.read_pixels(None).unwrap();
    if !cfg!(target_os = "visionos") {
        assert_eq!(s.read_pixel(20, 20).unwrap(), Color::new(40, 90, 140, 255));
        assert_eq!(s.read_pixel(5, 5).unwrap(), Color::new(100, 150, 200, 255));
    }
    r.present().unwrap();
}
