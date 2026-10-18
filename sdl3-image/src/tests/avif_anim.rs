//! The AVIF animation encoder against the reference harness: upstream
//! SDL_image encoding generated animations with libavif and libaom as
//! SDL_image's CMake builds them (libaom with `AOM_TARGET_CPU=generic`).
//!
//! libavif writes the current time as the creation and modification times
//! of the `mvhd`, `tkhd` and `mdhd` boxes of an image sequence, as the
//! translation does: the hashes are of the files with those fields zeroed.

use sdl3::io::IoStream;
use sdl3::properties::Properties;
use sdl3::video::pixels::PackedOrder;
use sdl3::video::{PixelFormat, Surface};

use super::{fnv, FNV0};
use crate::anim_decoder::*;
use crate::anim_encoder::*;

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        let mut r = self.0;
        r ^= r << 13;
        r ^= r >> 17;
        r ^= r << 5;
        self.0 = r;
        r
    }
}

fn hash2(a: u32, b: u32) -> u32 {
    let mut h = a.wrapping_mul(73856093) ^ b.wrapping_mul(19349663);
    h = h.wrapping_mul(2654435761);
    (h >> 24) & 0xff
}

/// A smooth texture at (u, v) in 1/16 pixel units, cells of 4 pixels.
fn tex(u: u32, v: u32, c: u32) -> u32 {
    let (gx, gy, fx, fy) = (u >> 6, v >> 6, u & 63, v & 63);
    let a = hash2(gx + c * 101, gy);
    let b = hash2(gx + 1 + c * 101, gy);
    let d = hash2(gx + c * 101, gy + 1);
    let e = hash2(gx + 1 + c * 101, gy + 1);
    let top = a * (64 - fx) + b * fx;
    let bot = d * (64 - fx) + e * fx;
    (top * (64 - fy) + bot * fy) >> 12
}

/// The harness's `gen()`: component `c` (0 red, 1 green, 2 blue, 3 alpha)
/// of frame `f` of a pattern (0 moving gradient, 1 noise, 2 moving shapes,
/// 3 still shapes, 4 moving texture, 5 scrolling screen content, 6 flat
/// windows, 7 scrolling text, 8 a moving checkerboard and stripes, 9 slides
/// changing every third frame; alpha 0 opaque, 1 moving gradient, 2 moving
/// binary, 3 noise).
#[allow(clippy::too_many_arguments)]
fn gen(
    rng: &mut Rng,
    pattern: i32,
    alpha: i32,
    c: i32,
    mut f: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    max: u32,
    amax: u32,
) -> u32 {
    if c == 3 {
        return match alpha {
            0 => amax,
            1 => ((((x + y + 3 * f) % (w + h)) as u64 * amax as u64) / (w + h - 1) as u64) as u32,
            2 => {
                if (((x + f) / 2 + y / 3) & 1) != 0 {
                    amax
                } else {
                    0
                }
            }
            _ => rng.next() % (amax + 1),
        };
    }
    match pattern {
        0 => {
            let (xx, yy) = (x + 2 * f, y + f);
            let d = if c == 0 {
                xx
            } else if c == 1 {
                yy
            } else {
                (xx + yy) / 2
            };
            let span = if c == 0 {
                w + 12
            } else if c == 1 {
                h + 6
            } else {
                (w + h + 18) / 2
            };
            ((d % span) as u64 * max as u64 / span as u64) as u32
        }
        1 => rng.next() % (max + 1),
        2 | 3 => {
            if pattern == 3 {
                f = 0;
            }
            let mut v = ((x + y) as u64 * max as u64 / (w + h) as u64) as u32 / 2 + max / 4;
            let (rx, ry) = ((f * 3) % w, (f * 2) % h);
            if x >= rx && x < rx + w / 3 + 1 && y >= ry && y < ry + h / 3 + 1 {
                v = if c == 0 {
                    max
                } else if c == 1 {
                    max / 5
                } else {
                    max / 2
                };
            }
            let (cx, cy, r) = (w - 1 - 2 * f, h / 2, w.min(h) / 4 + 1);
            if (x - cx) * (x - cx) + (y - cy) * (y - cy) <= r * r {
                v = if c == 0 {
                    max / 8
                } else if c == 1 {
                    max * 7 / 8
                } else {
                    max / 3
                };
            }
            v
        }
        4 => {
            tex(
                (x * 16 + f * 3 + 64) as u32,
                (y * 16 + f * 21 + 64) as u32,
                c as u32,
            ) * max
                / 255
        }
        6 => {
            // flat UI: windows of flat colors with 1-pixel borders, one moving
            const PAL: [[u8; 3]; 6] = [
                [240, 240, 240],
                [30, 30, 30],
                [0, 120, 215],
                [255, 255, 255],
                [200, 200, 200],
                [230, 80, 20],
            ];
            let mut idx = 0;
            let wx = [w / 8, w / 3 + 5 * f, w / 2 - 3 * f];
            let wy = [h / 6, h / 4 + 3 * f, h / 2];
            let ww = [w / 2, w / 3, w / 4 + 3];
            let wh = [h / 2, h / 3, h / 3];
            for k in 0..3 {
                let (xx, yy) = (x - wx[k], y - wy[k]);
                if xx >= 0 && yy >= 0 && xx < ww[k] && yy < wh[k] {
                    idx = if xx == 0 || yy == 0 || xx == ww[k] - 1 || yy == wh[k] - 1 {
                        1
                    } else if yy < 6 {
                        2 + (k == 2) as usize * 3
                    } else {
                        3 + (k == 1) as usize
                    };
                }
            }
            PAL[idx][c as usize] as u32 * max / 255
        }
        7 => {
            // text: 6x9 glyph cells, black on white, scrolling up a row a
            // frame, typed in
            let yy = y + f;
            let (cx, cy, gx, gy) = (x / 6, yy / 9, x % 6, yy % 9);
            let n = cy * 64 + cx;
            let mut on = false;
            if gx < 5
                && gy < 7
                && (cy * 13 + cx) < 40 + 25 * f
                && !hash2(n as u32, 7).is_multiple_of(9)
            {
                let bits =
                    hash2(n as u32, (gy * 5 + gx) as u32) ^ hash2((n % 37) as u32, gy as u32);
                on = (bits & 3) == 0 || ((gx == 0 || gy == 3) && (bits & 4) != 0);
            }
            let mut v = if on { 0 } else { 255 };
            if c == 2 && !on && cy % 4 == 0 {
                v = 230;
            }
            v * max / 255
        }
        8 => {
            // dither: a 1-pixel checkerboard of two colors and palette
            // stripes, moving left 2 pixels a frame
            let xx = x + 2 * f;
            if y < h / 2 {
                return if ((xx ^ y) & 1) != 0 {
                    max
                } else if c == 1 {
                    max / 2
                } else {
                    0
                };
            }
            const PAL: [[u8; 3]; 4] = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]];
            PAL[((xx / 3) & 3) as usize][c as usize] as u32 * max / 255
        }
        9 => {
            // slides: a new flat layout every third frame
            let sl = f / 3;
            let bg = hash2(sl as u32, 99);
            if y > h / 5 && y < h / 5 + 8 && x > w / 10 && x < w - w / 10 {
                return if c == sl % 3 { 255 } else { 20 } * max / 255;
            }
            if (x / 16 + y / 16 + sl) % 5 == 0 {
                return ((bg + 128 * (c == 0) as u32) & 255) * max / 255;
            }
            ((bg * (c as u32 + 1)) & 255) * max / 255
        }
        _ => {
            const PAL: [[u8; 3]; 4] = [[255, 255, 255], [0, 0, 0], [200, 30, 40], [20, 90, 220]];
            let k = ((x / 3) ^ ((y + 4 * f) / 5)) % 7;
            let idx = if k < 4 { 0 } else { (k - 4 + 1) as usize };
            PAL[idx][c as usize] as u32 * max / 255
        }
    }
}

/// The harness's `make()`: frame `f`.
fn make(
    rng: &mut Rng,
    w: i32,
    h: i32,
    fmt: PixelFormat,
    pattern: i32,
    alpha: i32,
    f: i32,
) -> Surface<'static> {
    if fmt.is_10bit() {
        let mut s = Surface::new(w, h, fmt).unwrap();
        let abgr = fmt.pixel_order() == PackedOrder::Abgr as u32
            || fmt.pixel_order() == PackedOrder::Xbgr as u32;
        let pitch = s.pitch() as usize;
        let pixels = s.pixels_mut().unwrap();
        for y in 0..h {
            for x in 0..w {
                let r = gen(rng, pattern, alpha, 0, f, x, y, w, h, 1023, 3);
                let g = gen(rng, pattern, alpha, 1, f, x, y, w, h, 1023, 3);
                let b = gen(rng, pattern, alpha, 2, f, x, y, w, h, 1023, 3);
                let a = gen(rng, pattern, alpha, 3, f, x, y, w, h, 1023, 3);
                let v = if abgr {
                    (a << 30) | (b << 20) | (g << 10) | r
                } else {
                    (a << 30) | (r << 20) | (g << 10) | b
                };
                let o = y as usize * pitch + x as usize * 4;
                pixels[o..o + 4].copy_from_slice(&v.to_ne_bytes());
            }
        }
        return s;
    }
    let mut s = Surface::new(w, h, PixelFormat::RGBA32).unwrap();
    let pitch = s.pitch() as usize;
    let pixels = s.pixels_mut().unwrap();
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                pixels[y as usize * pitch + x as usize * 4 + c as usize] =
                    gen(rng, pattern, alpha, c, f, x, y, w, h, 255, 255) as u8;
            }
        }
    }
    if fmt == PixelFormat::RGBA32 {
        return s;
    }
    s.convert(fmt).unwrap()
}

/// Zero the creation and modification times of the `mvhd`, `tkhd` and
/// `mdhd` boxes (version 1) in `moov` and the containers in it.
pub(super) fn normalize(p: &mut [u8], start: usize, end: usize, depth: i32) {
    let mut o = start;
    while o + 8 <= end {
        let sz = u32::from_be_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]]) as usize;
        if sz < 8 || o + sz > end {
            return;
        }
        let t = &p[o + 4..o + 8];
        if t == b"moov" || t == b"trak" || t == b"mdia" {
            normalize(p, o + 8, o + sz, depth + 1);
        } else if (t == b"mvhd" || t == b"tkhd" || t == b"mdhd")
            && depth > 0
            && sz >= 28
            && p[o + 8] == 1
        {
            p[o + 12..o + 28].fill(0);
        }
        o += sz;
    }
}

/// A case: `(name, width, height, format, pattern, alpha, frames,
/// durations, qualities, options)`; the options are the harness's: `m`
/// metadata, `l` loop count 3, `L` loop count 1, `k` key frame interval 2,
/// `t` time base 1/90000, `T` time base 3/100, `i` ignore the properties,
/// `1` one thread.
type Case = (
    &'static str,
    i32,
    i32,
    PixelFormat,
    i32,
    i32,
    i32,
    &'static str,
    &'static str,
    &'static str,
);

/// Encode case `i` at quality `q` with `threads` (the property, if any):
/// the file, or the error as the harness prints it.
fn encode(i: usize, c: &Case, q: i32, threads: Option<i64>) -> Result<Vec<u8>, String> {
    let &(_, w, h, fmt, pattern, alpha, frames, durations, _, opts) = c;
    let mut rng = Rng(0x2468ace0u32.wrapping_add(i as u32));
    let mut io = IoStream::from_dynamic_mem();
    let props = Properties::new();
    props
        .set(PROP_ANIMATION_ENCODER_CREATE_TYPE_STRING, "avifs")
        .unwrap();
    props
        .set(PROP_ANIMATION_ENCODER_CREATE_QUALITY_NUMBER, q as i64)
        .unwrap();
    if let Some(t) = threads {
        props
            .set(PROP_ANIMATION_ENCODER_CREATE_AVIF_MAX_THREADS_NUMBER, t)
            .unwrap();
    }
    for o in opts.chars() {
        match o {
            'm' => {
                props
                    .set(PROP_METADATA_TITLE_STRING, "Moving shapes")
                    .unwrap();
                props
                    .set(PROP_METADATA_AUTHOR_STRING, "sdl3-image tests")
                    .unwrap();
                props
                    .set(
                        PROP_METADATA_DESCRIPTION_STRING,
                        "An <animated> & generated test",
                    )
                    .unwrap();
                props
                    .set(PROP_METADATA_COPYRIGHT_STRING, "Public domain")
                    .unwrap();
                props
                    .set(PROP_METADATA_CREATION_TIME_STRING, "2025-01-02T03:04:05Z")
                    .unwrap();
            }
            'l' => props.set(PROP_METADATA_LOOP_COUNT_NUMBER, 3i64).unwrap(),
            'L' => props.set(PROP_METADATA_LOOP_COUNT_NUMBER, 1i64).unwrap(),
            'k' => props
                .set(
                    PROP_ANIMATION_ENCODER_CREATE_AVIF_KEYFRAME_INTERVAL_NUMBER,
                    2i64,
                )
                .unwrap(),
            't' => props
                .set(
                    PROP_ANIMATION_ENCODER_CREATE_TIMEBASE_DENOMINATOR_NUMBER,
                    90000i64,
                )
                .unwrap(),
            'T' => {
                props
                    .set(
                        PROP_ANIMATION_ENCODER_CREATE_TIMEBASE_NUMERATOR_NUMBER,
                        3i64,
                    )
                    .unwrap();
                props
                    .set(
                        PROP_ANIMATION_ENCODER_CREATE_TIMEBASE_DENOMINATOR_NUMBER,
                        100i64,
                    )
                    .unwrap();
            }
            'i' => props.set(PROP_METADATA_IGNORE_PROPS_BOOLEAN, true).unwrap(),
            '1' => props
                .set(PROP_ANIMATION_ENCODER_CREATE_AVIF_MAX_THREADS_NUMBER, 1i64)
                .unwrap(),
            _ => {}
        }
    }
    let mut encoder = AnimationEncoder::with_properties(Some(&mut io), &props)
        .map_err(|e| format!("create err: {e}"))?;
    let durations: Vec<u64> = durations.split(',').map(|d| d.parse().unwrap()).collect();
    for f in 0..frames {
        let mut s = make(&mut rng, w, h, fmt, pattern, alpha, f);
        encoder
            .add_frame(&mut s, durations[f as usize % durations.len()])
            .map_err(|e| format!("frame {f} err: {e}"))?;
    }
    encoder.close().map_err(|e| format!("close err: {e}"))?;
    Ok(io.dynamic_memory().unwrap().to_vec())
}

/// The harness's line for a result: `size hash` of the normalized file.
fn line(result: &Result<Vec<u8>, String>) -> String {
    match result {
        Ok(bytes) => {
            let mut b = bytes.clone();
            let n = b.len();
            normalize(&mut b, 0, n, 0);
            format!("{} {:016x}", n, fnv(FNV0, &b))
        }
        Err(e) => e.clone(),
    }
}

#[rustfmt::skip]
const CASES: &[Case] = &[
    ("rgbx_16x16_grad_f1", 16, 16, PixelFormat::RGBX32, 0, 0, 1, "100", "50", ""),
    ("rgbx_16x16_grad_f3", 16, 16, PixelFormat::RGBX32, 0, 0, 3, "100", "0,50,90,100", ""),
    ("rgba_17x9_shapes_a_f4", 17, 9, PixelFormat::RGBA32, 2, 1, 4, "40,0,250", "50,90", "m"),
    ("rgbx_33x21_tex_f6", 33, 21, PixelFormat::RGBX32, 4, 0, 6, "33", "25,75", "l"),
    ("rgba_24x24_noise_a_f2", 24, 24, PixelFormat::RGBA32, 1, 3, 2, "10,20", "50", ""),
    ("argb_40x30_shapes_bin_f5", 40, 30, PixelFormat::ARGB32, 2, 2, 5, "100", "60", "k"),
    ("rgbx_48x40_still_f4", 48, 40, PixelFormat::RGBX32, 3, 0, 4, "1000", "50", "L"),
    ("rgbx_64x64_screen_f3", 64, 64, PixelFormat::RGBX32, 5, 0, 3, "500", "50,90", "t"),
    ("xrgb2101010_20x12_tex_f3", 20, 12, PixelFormat::XRGB2101010, 4, 0, 3, "100", "50", ""),
    ("argb2101010_13x11_shapes_a_f4", 13, 11, PixelFormat::ARGB2101010, 2, 2, 4, "70", "80", "T"),
    ("argb4444_70x37_tex_a_f4", 70, 37, PixelFormat::ARGB4444, 4, 1, 4, "50", "40", "i"),
    ("rgba64_8x8_grad_f2", 8, 8, PixelFormat::RGBA64, 0, 1, 2, "100", "50", ""),
    ("rgbx_1x1_grad_f2", 1, 1, PixelFormat::RGBX32, 0, 0, 2, "100", "50", ""),
    ("rgba_3x7_noise_a_f3", 3, 7, PixelFormat::RGBA32, 1, 3, 3, "100", "50", "1"),
    ("rgbx_130x66_tex_f3", 130, 66, PixelFormat::RGBX32, 4, 0, 3, "40", "50", ""),
    ("rgba_24x24_noise_a_f1", 24, 24, PixelFormat::RGBA32, 1, 3, 1, "10", "50", ""),
    ("xrgb2101010_20x12_tex_f1", 20, 12, PixelFormat::XRGB2101010, 4, 0, 1, "100", "50", ""),
    ("rgbx_130x66_tex_f1", 130, 66, PixelFormat::RGBX32, 4, 0, 1, "40", "50,10", ""),
    ("rgbx_64x64_screen_f1", 64, 64, PixelFormat::RGBX32, 5, 0, 1, "500", "90", ""),
    ("argb2101010_13x11_shapes_a_f1", 13, 11, PixelFormat::ARGB2101010, 2, 2, 1, "70", "80", ""),
    ("rgbx_368x360_tex_f4", 368, 360, PixelFormat::RGBX32, 4, 0, 4, "100", "50", ""),
    ("rgba_400x376_shapes_a_f3", 400, 376, PixelFormat::RGBA32, 2, 1, 3, "100", "60", ""),
    ("rgbx_48x32_tex_f40", 48, 32, PixelFormat::RGBX32, 4, 0, 40, "40", "50", ""),
    ("rgbx_96x64_shapes_f12", 96, 64, PixelFormat::RGBX32, 2, 0, 12, "100", "30", ""),
    ("sc_rgbx_200x120_ui_f8", 200, 120, PixelFormat::RGBX32, 6, 0, 8, "100", "30", ""),
    ("sc_xrgb2101010_200x120_text_f8", 200, 120, PixelFormat::XRGB2101010, 7, 0, 8, "100", "60", "1"),
    ("sc_rgba_200x120_dither_a_f8", 200, 120, PixelFormat::RGBA32, 8, 2, 8, "100", "30", "k"),
    ("sc_rgbx_200x120_slides_f8", 200, 120, PixelFormat::RGBX32, 9, 0, 8, "100", "0,60", ""),
    ("sc_argb_200x120_slides_a_f8", 200, 120, PixelFormat::ARGB32, 9, 1, 8, "100", "30", "t"),
    ("sc_rgbx_704x480_text_f3", 704, 480, PixelFormat::RGBX32, 7, 0, 3, "100", "50", ""),
    ("lg_rgbx_1000x700_slides_f7", 1000, 700, PixelFormat::RGBX32, 9, 0, 7, "100", "70", ""),
    ("lg_argb_1080x1080_text_a_f2", 1080, 1080, PixelFormat::ARGB32, 7, 1, 2, "100", "35", ""),
    ("lg_xrgb2101010_1080x1088_ui_f2", 1080, 1088, PixelFormat::XRGB2101010, 6, 0, 2, "100", "60", ""),
    ("lg_rgbx_4200x64_ui_f2", 4200, 64, PixelFormat::RGBX32, 6, 0, 2, "100", "50", ""),
];

/// Development aid: print the harness's lines for the cases whose label
/// contains `AVIF_DEV_FILTER`, writing the files to `AVIF_DEV_OUT` if set
/// (threads from `AVIF_THREADS`, as the harness's).
#[test]
#[ignore]
fn avif_anim_dev() {
    let filter = std::env::var("AVIF_DEV_FILTER").unwrap_or_default();
    let out = std::env::var("AVIF_DEV_OUT").ok();
    let threads = std::env::var("AVIF_THREADS")
        .ok()
        .map(|t| t.parse().unwrap());
    for (i, c) in CASES.iter().enumerate() {
        for q in c.8.split(',') {
            let label = format!("anim {} q{}", c.0, q);
            if !label.contains(&filter) {
                continue;
            }
            let r = encode(i, c, q.parse().unwrap(), threads);
            if let (Some(out), Ok(bytes)) = (&out, &r) {
                std::fs::write(format!("{out}/{}.avifs", label.replace(' ', "_")), bytes).unwrap();
            }
            println!("{label}: {}", line(&r));
        }
    }
}

/// The decoded frames of an encoded case against its source frames: their
/// count, size and delays, and their pixels within the quality's error.
fn check_round_trip(i: usize, c: &Case, q: i32, bytes: &[u8]) -> Result<(), String> {
    let &(name, w, h, fmt, pattern, alpha, frames, durations, _, opts) = c;
    let anim = crate::load_animation_typed_io(&mut IoStream::from_const_mem(bytes), Some("avifs"))
        .map_err(|e| format!("{name} q{q}: decode: {e}"))?;
    if anim.count() != frames as usize || anim.w != w || anim.h != h {
        return Err(format!(
            "{name} q{q}: decoded {} {}x{}, not {frames} {w}x{h}",
            anim.count(),
            anim.w,
            anim.h
        ));
    }
    let durations: Vec<i32> = durations.split(',').map(|d| d.parse().unwrap()).collect();
    let mut rng = Rng(0x2468ace0u32.wrapping_add(i as u32));
    for f in 0..frames as usize {
        // (the default time base, milliseconds, is the decoder's; a single
        // frame is written as a still image, which decodes with a delay of
        // 1, as do the frames of duration 0)
        let duration = if frames == 1 {
            1
        } else {
            durations[f % durations.len()].max(1)
        };
        if !opts.contains('t') && !opts.contains('T') && anim.delays[f] != duration {
            return Err(format!(
                "{name} q{q}: frame {f} delay {}, not {duration}",
                anim.delays[f]
            ));
        }
        let src = make(&mut rng, w, h, fmt, pattern, alpha, f as i32)
            .convert(PixelFormat::RGBA32)
            .unwrap();
        // (upstream's 10-bit frames lose their alpha: they decode as
        // XBGR2101010, compared without it)
        let with_alpha = anim.frames[f].format().has_alpha();
        let dec = anim.frames[f].convert(PixelFormat::RGBA32).unwrap();
        let (sp, dp) = (src.pitch() as usize, dec.pitch() as usize);
        let (sb, db) = (src.pixels().unwrap(), dec.pixels().unwrap());
        // (the colors weighed by their alpha)
        let mut err = 0u64;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let (s, d) = (&sb[y * sp + x * 4..][..4], &db[y * dp + x * 4..][..4]);
                let (sa, da) = if with_alpha {
                    (s[3] as i64, d[3] as i64)
                } else {
                    (255, 255)
                };
                for k in 0..3 {
                    let (a, b) = (s[k] as i64 * sa, d[k] as i64 * da);
                    err += ((a - b).unsigned_abs() + 127) / 255;
                }
                err += (sa - da).unsigned_abs();
            }
        }
        let mean = err as f64 / (w * h * 4) as f64;
        // (the noise patterns keep little of themselves, the others less at
        // low qualities)
        let bound = if pattern == 1 || alpha == 3 {
            64.0
        } else if q < 50 {
            48.0
        } else {
            24.0
        };
        if mean > bound {
            return Err(format!("{name} q{q}: frame {f} mean error {mean}"));
        }
    }
    Ok(())
}

/// Whether case `c` is checked with `threads_label`: every case with the
/// default threads and one thread, and with four threads the cases up to
/// 100x100 (four threads code the same files as the default two: with more
/// than one, libaom's row multi-threading codes the same rows with the same
/// contexts whatever the number of workers); the debug builds, whose
/// encodes are many times slower, only the cases up to 100x100 with the
/// default threads and one thread.
fn checked(c: &Case, threads_label: &str) -> bool {
    let small = c.1 * c.2 <= 100 * 100;
    if cfg!(debug_assertions) {
        small && threads_label != "4"
    } else {
        small || threads_label != "4"
    }
}

/// Every case, with the default threads (as on 4 logical cores: see
/// `avif::logical_cpu_cores()`), one thread and 4 threads, against the
/// reference harness's results; the default threads' files also decode
/// back to their frames (see `checked()` for the subsets checked).
#[test]
fn avif_animations_match_reference() {
    let reference = include_str!("../testdata/avif_anim_reference.txt");
    let mut expected: Vec<(&str, Vec<&str>)> = Vec::new();
    for l in reference.lines() {
        if let Some(t) = l.strip_prefix("[threads ") {
            expected.push((t.trim_end_matches(']'), Vec::new()));
        } else if !l.starts_with('#') && !l.is_empty() {
            expected.last_mut().unwrap().1.push(l);
        }
    }
    let mut failures = Vec::new();
    let mut round_trips = 0;
    for (threads_label, lines) in &expected {
        let threads = match *threads_label {
            "default" => None,
            t => Some(t.parse().unwrap()),
        };
        let mut k = 0;
        for (i, c) in CASES.iter().enumerate() {
            for q in c.8.split(',') {
                if !checked(c, threads_label) {
                    k += 1;
                    continue;
                }
                let q: i32 = q.parse().unwrap();
                let r = encode(i, c, q, threads);
                let actual = format!("anim {} q{q}: {}", c.0, line(&r));
                if lines.get(k) != Some(&actual.as_str()) {
                    failures.push(format!(
                        "threads {threads_label}:\n  expected {:?}\n  actual   {actual}",
                        lines.get(k)
                    ));
                }
                k += 1;
                if let (None, Ok(bytes)) = (threads, &r) {
                    if let Err(e) = check_round_trip(i, c, q, bytes) {
                        failures.push(e);
                    }
                    round_trips += 1;
                }
            }
        }
        assert_eq!(
            k,
            lines.len(),
            "threads {threads_label}: the reference's lines"
        );
    }
    assert_eq!(expected.len(), 3);
    assert!(round_trips > 15);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
