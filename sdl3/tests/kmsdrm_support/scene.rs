// The renderer frame the KMS/DRM driver's programs draw, and its
// comparison with the software renderer's.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use sdl3::render::{Renderer, Texture};
use sdl3::video::{BlendMode, FRect, PixelFormat, Rect, Surface};

/// A 16x16 texture pattern (the one the renderer scenes use).
pub fn pattern() -> Surface<'static> {
    let mut s = Surface::new(16, 16, PixelFormat::ARGB8888).unwrap();
    for y in 0..16 {
        for x in 0..16 {
            let color = s.map_rgb(
                (x * 16) as u8,
                (y * 16) as u8,
                if (x + y) % 2 == 0 { 255 } else { 0 },
            );
            s.fill_rect(Some(&Rect::new(x, y, 1, 1)), color).unwrap();
        }
    }
    s
}

/// A frame of the renderers: a clear, filled rectangles (one moving
/// with the frame), a translucent one blended, the texture copied 1:1.
pub fn draw_frame(r: &mut Renderer, frame: u8, texture: Option<Texture>) {
    r.set_draw_color(32, 64 + frame * 64, 96, 255);
    let _ = r.clear();
    r.set_draw_color(255, 0, 0, 255);
    let _ = r.render_fill_rect(Some(&FRect::new(50.0, 50.0, 100.0, 100.0)));
    r.set_draw_color(0, 255, 0, 255);
    let _ = r.render_fill_rect(Some(&FRect::new(250.0, 150.0, 100.0, 100.0)));
    r.set_draw_color(0, 0, 255, 255);
    let _ = r.render_fill_rect(Some(&FRect::new(
        450.0 + frame as f32 * 50.0,
        334.0,
        124.0,
        100.0,
    )));
    let _ = r.set_draw_blend_mode(BlendMode::BLEND);
    r.set_draw_color(255, 255, 255, 128);
    let _ = r.render_fill_rect(Some(&FRect::new(100.0, 100.0, 200.0, 100.0)));
    let _ = r.set_draw_blend_mode(BlendMode::NONE);
    if let Some(texture) = texture {
        let _ = r.render_texture(texture, None, Some(&FRect::new(692.0, 492.0, 16.0, 16.0)));
    }
}

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
pub fn fnv(bytes: &[u8]) -> u32 {
    let mut h: u32 = 2166136261;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

/// The pixels of a frame read back (`rgba()`'s) that differ from the
/// software renderer's frame of the same size by more than 1 in a
/// channel (`None`: the software renderer's has another size).
pub fn differing_from_software(image: &(i32, i32, Vec<u8>), frame: u8) -> Option<usize> {
    let mut sw =
        Renderer::software(Surface::new(image.0, image.1, PixelFormat::ARGB8888).unwrap()).unwrap();
    let sw_texture = sw.create_texture_from_surface(&mut pattern()).ok();
    draw_frame(&mut sw, frame, sw_texture);
    let theirs = rgba(&sw.read_pixels(None).unwrap());
    if (image.0, image.1) != (theirs.0, theirs.1) {
        return None;
    }
    Some(
        image
            .2
            .chunks(4)
            .zip(theirs.2.chunks(4))
            .filter(|(a, b)| (0..3).any(|c| a[c].abs_diff(b[c]) > 1))
            .count(),
    )
}
