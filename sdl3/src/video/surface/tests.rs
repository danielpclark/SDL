// Tests for the surface module. The `*_matches_c` tests replay the same
// operations as a harness built from upstream's C sources and compare
// FNV-1a hashes of every result, so any difference in any pixel, error or
// attribute changes the hash. The C side is built with allocations zeroed
// like Rust's, and is run with its x86 SIMD kernels both enabled and
// disabled.

#![allow(clippy::needless_range_loop)] // the loops mirror the C harness

use super::*;
use crate::video::pixels::Color;

struct Harness {
    rng: u64,
    h: u64,
    debug: bool,
}

const FMTS: [PixelFormat; 54] = [
    PixelFormat::INDEX1LSB,
    PixelFormat::INDEX1MSB,
    PixelFormat::INDEX2LSB,
    PixelFormat::INDEX2MSB,
    PixelFormat::INDEX4LSB,
    PixelFormat::INDEX4MSB,
    PixelFormat::INDEX8,
    PixelFormat::RGB332,
    PixelFormat::XRGB4444,
    PixelFormat::XBGR4444,
    PixelFormat::XRGB1555,
    PixelFormat::XBGR1555,
    PixelFormat::ARGB4444,
    PixelFormat::RGBA4444,
    PixelFormat::ABGR4444,
    PixelFormat::BGRA4444,
    PixelFormat::ARGB1555,
    PixelFormat::RGBA5551,
    PixelFormat::ABGR1555,
    PixelFormat::BGRA5551,
    PixelFormat::RGB565,
    PixelFormat::BGR565,
    PixelFormat::RGB24,
    PixelFormat::BGR24,
    PixelFormat::XRGB8888,
    PixelFormat::RGBX8888,
    PixelFormat::XBGR8888,
    PixelFormat::BGRX8888,
    PixelFormat::ARGB8888,
    PixelFormat::RGBA8888,
    PixelFormat::ABGR8888,
    PixelFormat::BGRA8888,
    PixelFormat::XRGB2101010,
    PixelFormat::XBGR2101010,
    PixelFormat::ARGB2101010,
    PixelFormat::ABGR2101010,
    PixelFormat::RGB48,
    PixelFormat::BGR48,
    PixelFormat::RGBA64,
    PixelFormat::ARGB64,
    PixelFormat::BGRA64,
    PixelFormat::ABGR64,
    PixelFormat::RGB48_FLOAT,
    PixelFormat::BGR48_FLOAT,
    PixelFormat::RGBA64_FLOAT,
    PixelFormat::ARGB64_FLOAT,
    PixelFormat::BGRA64_FLOAT,
    PixelFormat::ABGR64_FLOAT,
    PixelFormat::RGB96_FLOAT,
    PixelFormat::BGR96_FLOAT,
    PixelFormat::RGBA128_FLOAT,
    PixelFormat::ARGB128_FLOAT,
    PixelFormat::BGRA128_FLOAT,
    PixelFormat::ABGR128_FLOAT,
];
/// Indices into FMTS for the blit tests.
const SUB: [usize; 27] = [
    0, 2, 4, 6, 7, 8, 10, 12, 16, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 38,
    42, 50,
];

const MODES: [BlendMode; 7] = [
    BlendMode::NONE,
    BlendMode::BLEND,
    BlendMode::BLEND_PREMULTIPLIED,
    BlendMode::ADD,
    BlendMode::ADD_PREMULTIPLIED,
    BlendMode::MOD,
    BlendMode::MUL,
];

impl Harness {
    fn new() -> Harness {
        Harness {
            rng: 1,
            h: 0,
            debug: std::env::var_os("SDL_SURFACE_TEST_DEBUG").is_some(),
        }
    }
    fn rnd(&mut self) -> u32 {
        self.rng = self
            .rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.rng >> 33) as u32
    }
    fn rnd_float(&mut self) -> f32 {
        (self.rnd() % 1500) as f32 / 1000.0 - 0.25
    }
    fn reset(&mut self) {
        self.h = 0xcbf29ce484222325;
    }
    fn hb(&mut self, b: &[u8]) {
        for &x in b {
            self.h ^= x as u64;
            self.h = self.h.wrapping_mul(0x100000001b3);
        }
    }
    fn h8(&mut self, v: u8) {
        self.hb(&[v]);
    }
    fn hok<T>(&mut self, r: &Result<T>) {
        self.h8(r.is_ok() as u8);
    }
    fn h32(&mut self, v: u32) {
        self.hb(&v.to_le_bytes());
    }
    fn hf(&mut self, v: f32) {
        self.hb(&v.to_ne_bytes());
    }
    fn dbg(&self, what: &str, a: usize, b: usize, c: usize) {
        if self.debug {
            println!("  {what} {a} {b} {c} {:016x}", self.h);
        }
    }

    fn fill_random(&mut self, s: &mut Surface<'_>) {
        let f = s.format;
        let n = (s.h * s.pitch) as usize;
        let Some(_) = s.pixels.bytes() else { return };
        let mut bytes = vec![0u8; n];
        if f.pixel_type() == crate::video::pixels::PixelType::ArrayF32 {
            for i in 0..n / 4 {
                let v = self.rnd_float();
                bytes[4 * i..4 * i + 4].copy_from_slice(&v.to_ne_bytes());
            }
        } else if f.pixel_type() == crate::video::pixels::PixelType::ArrayF16 {
            for i in 0..n / 2 {
                let v = crate::video::blit::slow::float_to_half(self.rnd_float());
                bytes[2 * i..2 * i + 2].copy_from_slice(&v.to_ne_bytes());
            }
        } else {
            for b in bytes.iter_mut() {
                *b = self.rnd() as u8;
            }
        }
        s.pixels.bytes_mut().unwrap()[..n].copy_from_slice(&bytes);
        if f.is_indexed() {
            let pal = s.create_palette().unwrap();
            let ncolors = read_palette(&pal).len();
            for i in 0..ncolors {
                let r = self.rnd() as u8;
                let g = self.rnd() as u8;
                let b = self.rnd() as u8;
                let a = if self.rnd() & 1 != 0 {
                    255
                } else {
                    self.rnd() as u8
                };
                write_palette(&pal)
                    .set_colors(i, &[Color::new(r, g, b, a)])
                    .unwrap();
            }
        }
    }

    fn hash_surface(&mut self, s: Option<&Surface<'_>>) {
        let Some(s) = s else {
            self.h8(0xEE);
            return;
        };
        self.h32(s.format.0);
        self.h32(s.w as u32);
        self.h32(s.h as u32);
        let Some(px) = s.pixels.bytes() else {
            self.h8(0xDD);
            return;
        };
        let bits = s.format.bits_per_pixel() as i32;
        let row = if bits >= 8 {
            s.w * s.format.bytes_per_pixel() as i32
        } else {
            (s.w * bits + 7) / 8
        } as usize;
        let px = px.to_vec();
        for y in 0..s.h as usize {
            let start = y * s.pitch as usize;
            self.hb(&px[start..start + row]);
        }
        if let Some(p) = &s.palette {
            let colors: Vec<Color> = read_palette(p).colors().to_vec();
            for c in colors {
                self.hb(&[c.r, c.g, c.b, c.a]);
            }
        }
    }
}

fn first_pixel(s: &Surface<'_>) -> u32 {
    let px = s.pixels.bytes().unwrap();
    if s.format.bits_per_pixel() < 8 {
        return (px[0] & 1) as u32;
    }
    let bpp = (s.format.bytes_per_pixel() as usize).min(4);
    let mut b = [0u8; 4];
    b[..bpp].copy_from_slice(&px[..bpp]);
    u32::from_ne_bytes(b)
}

fn colorkey_for(t: &mut Harness, s: &Surface<'_>) -> u32 {
    match &s.palette {
        Some(p) => {
            let n = read_palette(p).len() as u32;
            t.rnd() % n
        }
        None => first_pixel(s),
    }
}

/// 1: SDL_ConvertSurface between every pair of formats
fn run_convert(t: &mut Harness) -> u64 {
    t.reset();
    for i in 0..FMTS.len() {
        for j in 0..FMTS.len() {
            let mut s = Surface::new(7, 5, FMTS[i]).unwrap();
            t.fill_random(&mut s);
            let r = t.rnd();
            /* colorkeys converted to >4-byte formats overflow an int in upstream (UB) */
            if r % 3 == 0 && FMTS[j].bytes_per_pixel() <= 4 {
                let key = colorkey_for(t, &s);
                s.set_color_key(Some(key)).unwrap();
            }
            if r % 5 == 1 {
                let a = t.rnd() as u8;
                s.set_alpha_mod(a);
            }
            if r % 7 == 2 {
                let (cr, cg, cb) = (t.rnd() as u8, t.rnd() as u8, t.rnd() as u8);
                s.set_color_mod(cr, cg, cb);
            }
            let d = s.convert(FMTS[j]);
            t.hok(&d);
            let d = d.ok();
            t.hash_surface(d.as_ref());
            if let Some(d) = &d {
                let key = d.color_key();
                t.h8(key.is_some() as u8);
                t.h32(key.unwrap_or(0));
                t.h32(d.blend_mode().0);
                t.h8(d.alpha_mod());
            }
            t.dbg("convert", i, j, 0);
        }
    }
    t.h
}

/// 2: SDL_BlitSurface with every blend mode, modulation and colorkey
fn run_blit(t: &mut Harness) -> u64 {
    t.reset();
    for i in 0..SUB.len() {
        for j in 0..SUB.len() {
            for (m, mode) in MODES.iter().enumerate() {
                for v in 0..4 {
                    // SDL_blit_0.c drifts by leading_skip bytes per row (see
                    // walk_bits); taller bitmaps keep the C side's reads inside
                    // the buffer, where both sides read the same bytes.
                    let sh = if FMTS[SUB[i]].bits_per_pixel() < 8 {
                        40
                    } else {
                        7
                    };
                    let mut s = Surface::new(9, sh, FMTS[SUB[i]]).unwrap();
                    let mut d = Surface::new(13, 11, FMTS[SUB[j]]).unwrap();
                    t.fill_random(&mut s);
                    t.fill_random(&mut d);
                    s.set_blend_mode(*mode).unwrap();
                    if v & 1 != 0 {
                        let (cr, cg, cb) = (t.rnd() as u8, t.rnd() as u8, t.rnd() as u8);
                        s.set_color_mod(cr, cg, cb);
                        let a = t.rnd() as u8;
                        s.set_alpha_mod(a);
                    }
                    if v & 2 != 0 {
                        let key = colorkey_for(t, &s);
                        s.set_color_key(Some(key)).unwrap();
                    }
                    if t.rnd() % 4 == 0 {
                        let x = (t.rnd() % 5) as i32;
                        let y = (t.rnd() % 5) as i32;
                        let w = (t.rnd() % 10) as i32;
                        let h = (t.rnd() % 10) as i32;
                        d.set_clip_rect(Some(&Rect::new(x, y, w, h)));
                    }
                    let sr = Rect::new(
                        (t.rnd() % 5) as i32 - 1,
                        (t.rnd() % 5) as i32 - 1,
                        (t.rnd() % 10) as i32,
                        (t.rnd() % 8) as i32,
                    );
                    let dr = Rect::new((t.rnd() % 17) as i32 - 4, (t.rnd() % 15) as i32 - 4, 0, 0);
                    let use_sr = t.rnd() & 1 != 0;
                    let ok = s.blit(use_sr.then_some(&sr), &mut d, Some(&dr));
                    t.hok(&ok);
                    t.hash_surface(Some(&d));
                    /* blit again through the cached map */
                    let ok = s.blit(None, &mut d, None);
                    t.hok(&ok);
                    t.hash_surface(Some(&d));
                    t.dbg("blit", i * 100 + j, m, v);
                }
            }
        }
    }
    t.h
}

/// 3: SDL_BlitSurfaceScaled
fn run_scaled(t: &mut Harness) -> u64 {
    const SM: [ScaleMode; 3] = [ScaleMode::Nearest, ScaleMode::Linear, ScaleMode::PixelArt];
    t.reset();
    for i in 0..SUB.len() {
        for j in 0..SUB.len() {
            for (m, sm) in SM.iter().enumerate() {
                for v in 0..4 {
                    let mut s = Surface::new(7, 5, FMTS[SUB[i]]).unwrap();
                    let mut d = Surface::new(19, 17, FMTS[SUB[j]]).unwrap();
                    t.fill_random(&mut s);
                    t.fill_random(&mut d);
                    s.set_blend_mode(if v & 1 != 0 {
                        BlendMode::BLEND
                    } else {
                        BlendMode::NONE
                    })
                    .unwrap();
                    if v & 2 != 0 {
                        let (cr, cg, cb) = (t.rnd() as u8, t.rnd() as u8, t.rnd() as u8);
                        s.set_color_mod(cr, cg, cb);
                    }
                    let sr = Rect::new(
                        (t.rnd() % 4) as i32 - 1,
                        (t.rnd() % 4) as i32 - 1,
                        1 + (t.rnd() % 7) as i32,
                        1 + (t.rnd() % 5) as i32,
                    );
                    let dr = Rect::new(
                        (t.rnd() % 12) as i32 - 3,
                        (t.rnd() % 12) as i32 - 3,
                        1 + (t.rnd() % 20) as i32,
                        1 + (t.rnd() % 20) as i32,
                    );
                    let r = t.rnd();
                    let ok = s.blit_scaled(
                        (r & 1 != 0).then_some(&sr),
                        &mut d,
                        (r & 2 != 0).then_some(&dr),
                        *sm,
                    );
                    t.hok(&ok);
                    t.hash_surface(Some(&d));
                    t.dbg("scaled", i * 100 + j, m, v);
                }
            }
        }
    }
    t.h
}

/// 4: fills, clear, flip, premultiply, pixel access, duplicate, scale, tiled and 9-grid blits
fn run_misc(t: &mut Harness) -> u64 {
    t.reset();
    for (i, &f) in FMTS.iter().enumerate() {
        let mut s = Surface::new(11, 9, f).unwrap();
        t.fill_random(&mut s);
        let mut rects = [Rect::default(); 3];
        for r in rects.iter_mut() {
            r.x = (t.rnd() % 14) as i32 - 2;
            r.y = (t.rnd() % 12) as i32 - 2;
            r.w = (t.rnd() % 9) as i32;
            r.h = (t.rnd() % 9) as i32;
        }
        let c = t.rnd();
        let ok = s.fill_rects(&rects, c);
        t.hok(&ok);
        t.hash_surface(Some(&s));
        t.dbg("fillrects", i, 0, 0);
        let c = t.rnd();
        let ok = s.fill_rect(None, c);
        t.hok(&ok);
        t.hash_surface(Some(&s));
        t.dbg("fillrect", i, 0, 0);
        t.fill_random(&mut s);
        let (cr, cg, cb, ca) = (t.rnd_float(), t.rnd_float(), t.rnd_float(), t.rnd_float());
        let ok = s.clear(cr, cg, cb, ca);
        t.hok(&ok);
        t.hash_surface(Some(&s));
        t.dbg("fill", i, 0, 0);
        t.fill_random(&mut s);
        for mode in [
            FlipMode::Horizontal,
            FlipMode::Vertical,
            FlipMode::HorizontalAndVertical,
        ] {
            let ok = s.flip(mode);
            t.hok(&ok);
            t.hash_surface(Some(&s));
        }
        let ok = s.flip(FlipMode::None);
        t.hok(&ok);
        t.dbg("flip", i, 0, 0);
        let ok = s.premultiply_alpha(false);
        t.hok(&ok);
        t.hash_surface(Some(&s));
        t.fill_random(&mut s);
        let ok = s.premultiply_alpha(true);
        t.hok(&ok);
        t.hash_surface(Some(&s));
        t.dbg("premul", i, 0, 0);
        for _ in 0..6 {
            let x = (t.rnd() % 13) as i32 - 1;
            let y = (t.rnd() % 11) as i32 - 1;
            let c = s.read_pixel(x, y);
            t.hok(&c);
            let c = c.unwrap_or(Color::new(0, 0, 0, 0));
            t.hb(&[c.r, c.g, c.b, c.a]);
            let fc = s.read_pixel_float(x, y);
            t.hok(&fc);
            let fc = fc.unwrap_or(FColor::new(0.0, 0.0, 0.0, 0.0));
            t.hf(fc.r);
            t.hf(fc.g);
            t.hf(fc.b);
            t.hf(fc.a);
            let (wr, wg, wb, wa) = (t.rnd() as u8, t.rnd() as u8, t.rnd() as u8, t.rnd() as u8);
            let ok = s.write_pixel(x, y, Color::new(wr, wg, wb, wa));
            t.hok(&ok);
            let (wr, wg, wb, wa) = (t.rnd_float(), t.rnd_float(), t.rnd_float(), t.rnd_float());
            let ok = s.write_pixel_float((x + 3) % 11, y, FColor::new(wr, wg, wb, wa));
            t.hok(&ok);
            let (mr, mg, mb, ma) = (t.rnd() as u8, t.rnd() as u8, t.rnd() as u8, t.rnd() as u8);
            let px = s.map_rgba(mr, mg, mb, ma);
            t.h32(px);
        }
        t.hash_surface(Some(&s));
        t.dbg("pixel", i, 0, 0);
        let d = s.duplicate().ok();
        t.hash_surface(d.as_ref());
        for m in 0..2 {
            let sw = 1 + (t.rnd() % 23) as i32;
            let sh = 1 + (t.rnd() % 19) as i32;
            let d = s
                .scale(
                    sw,
                    sh,
                    if m != 0 {
                        ScaleMode::Linear
                    } else {
                        ScaleMode::Nearest
                    },
                )
                .ok();
            t.hash_surface(d.as_ref());
        }
        t.dbg("scale", i, 0, 0);
        let mut tt = Surface::new(29, 23, PixelFormat::ARGB8888).unwrap();
        t.fill_random(&mut tt);
        let sr = Rect::new(1, 2, 5, 4);
        let dr = Rect::new(2, 1, 25, 20);
        let ok = s.blit_tiled(Some(&sr), &mut tt, Some(&dr));
        t.hok(&ok);
        t.hash_surface(Some(&tt));
        let ok = s.blit_tiled_with_scale(Some(&sr), 1.5, ScaleMode::Nearest, &mut tt, None);
        t.hok(&ok);
        t.hash_surface(Some(&tt));
        let ok = s.blit_9grid(None, 3, 2, 2, 3, 2.0, ScaleMode::Linear, &mut tt, Some(&dr));
        t.hok(&ok);
        t.hash_surface(Some(&tt));
        let ok = s.blit_9grid(
            Some(&sr),
            1,
            1,
            1,
            1,
            0.0,
            ScaleMode::Nearest,
            &mut tt,
            None,
        );
        t.hok(&ok);
        t.hash_surface(Some(&tt));
        t.dbg("tiled", i, 0, 0);
    }
    t.h
}

/// 5: blits between colorspaces, with HDR properties
fn run_colorspace(t: &mut Harness) -> u64 {
    const CS: [Colorspace; 4] = [
        Colorspace::SRGB,
        Colorspace::SRGB_LINEAR,
        Colorspace::HDR10,
        Colorspace::BT709_FULL,
    ];
    const CF: [usize; 11] = [7, 20, 22, 28, 29, 34, 38, 42, 44, 50, 51];
    const TM: [Option<&str>; 5] = [
        None,
        Some("chrome"),
        Some("*=0.5"),
        Some("none"),
        Some("bogus"),
    ];
    t.reset();
    for i in 0..11 {
        for j in 0..11 {
            for a in 0..4 {
                for b in 0..4 {
                    let mut s = Surface::new(5, 3, FMTS[CF[i]]).unwrap();
                    let mut d = Surface::new(5, 3, FMTS[CF[j]]).unwrap();
                    t.fill_random(&mut s);
                    t.fill_random(&mut d);
                    s.set_colorspace(CS[a]);
                    d.set_colorspace(CS[b]);
                    let r = t.rnd();
                    if r & 1 != 0 {
                        let v = 100.0 + (t.rnd() % 200) as f32;
                        s.properties()
                            .set(PROP_SURFACE_SDR_WHITE_POINT_FLOAT, v)
                            .unwrap();
                    }
                    if r & 2 != 0 {
                        let v = 1.0 + (t.rnd() % 8) as f32;
                        s.properties()
                            .set(PROP_SURFACE_HDR_HEADROOM_FLOAT, v)
                            .unwrap();
                    }
                    if r & 4 != 0 {
                        let v = 1.0 + (t.rnd() % 4) as f32;
                        d.properties()
                            .set(PROP_SURFACE_HDR_HEADROOM_FLOAT, v)
                            .unwrap();
                    }
                    let tm = TM[(t.rnd() % 5) as usize];
                    if let Some(tm) = tm {
                        s.properties()
                            .set(PROP_SURFACE_TONEMAP_OPERATOR_STRING, tm)
                            .unwrap();
                    }
                    s.set_blend_mode(if r & 8 != 0 {
                        BlendMode::BLEND
                    } else {
                        BlendMode::NONE
                    })
                    .unwrap();
                    let ok = s.blit(None, &mut d, None);
                    t.hok(&ok);
                    t.hash_surface(Some(&d));
                    t.h32((d.hdr_headroom(CS[b]) * 1000.0) as u32);
                    t.dbg("cs", i * 100 + j, a, b);
                }
            }
        }
    }
    t.h
}

/// 6: RLE-accelerated blits
fn run_rle(t: &mut Harness) -> u64 {
    const RF: [usize; 11] = [6, 7, 10, 20, 22, 24, 28, 29, 30, 31, 16];
    const RD: [usize; 7] = [20, 10, 24, 28, 30, 22, 6];
    t.reset();
    for i in 0..11 {
        for j in 0..7 {
            for v in 0..8 {
                let w = if v == 7 { 300 } else { 23 };
                let mut s = Surface::new(w, 13, FMTS[RF[i]]).unwrap();
                let mut d = Surface::new(w + 7, 17, FMTS[RD[j]]).unwrap();
                t.fill_random(&mut s);
                t.fill_random(&mut d);
                let key = first_pixel(&s);
                let bpp = s.format.bytes_per_pixel() as usize;
                let pitch = s.pitch as usize;
                for y in 0..s.h {
                    for x in 0..s.w {
                        let r = t.rnd() % 4;
                        let p = y as usize * pitch + x as usize * bpp;
                        if r == 0 {
                            let px = s.pixels_mut().unwrap();
                            let first: Vec<u8> = px[..bpp].to_vec();
                            px[p..p + bpp].copy_from_slice(&first);
                        } else if s.format.has_alpha() {
                            let c = s.read_pixel(x, y).unwrap();
                            let a = match r {
                                1 => 0,
                                2 => 255,
                                _ => t.rnd() as u8,
                            };
                            s.write_pixel(x, y, Color::new(c.r, c.g, c.b, a)).unwrap();
                        }
                    }
                }
                let ok = s.set_rle(true);
                t.hok(&ok);
                if v & 1 != 0 {
                    let k = match &s.palette {
                        Some(p) => key % read_palette(p).len() as u32,
                        None => key,
                    };
                    s.set_color_key(Some(k)).unwrap();
                }
                s.set_blend_mode(if v & 2 != 0 {
                    BlendMode::BLEND
                } else {
                    BlendMode::NONE
                })
                .unwrap();
                if v & 4 != 0 {
                    let a = if v & 1 != 0 { 128 } else { t.rnd() as u8 };
                    s.set_alpha_mod(a);
                }
                let mut dr = Rect::new((t.rnd() % 9) as i32 - 4, (t.rnd() % 9) as i32 - 4, 0, 0);
                let ok = s.blit(None, &mut d, Some(&dr));
                t.hok(&ok);
                t.h8(s.must_lock() as u8);
                t.hash_surface(Some(&d));
                let sr = Rect::new(
                    (t.rnd() % 7) as i32,
                    (t.rnd() % 5) as i32,
                    (t.rnd() % w as u32) as i32,
                    (t.rnd() % 13) as i32,
                );
                dr.x = (t.rnd() % 9) as i32 - 2;
                dr.y = (t.rnd() % 9) as i32 - 2;
                let ok = s.blit(Some(&sr), &mut d, Some(&dr));
                t.hok(&ok);
                t.hash_surface(Some(&d));
                {
                    let lock = s.lock();
                    t.hok(&lock);
                    let lock = lock.unwrap();
                    t.hash_surface(Some(lock.surface()));
                }
                t.h8(s.must_lock() as u8);
                let ok = s.blit(Some(&sr), &mut d, None);
                t.hok(&ok);
                t.h8(s.must_lock() as u8);
                t.hash_surface(Some(&d));
                t.dbg("rle", i * 100 + j, v, 0);
            }
        }
    }
    t.h
}

/// 7: SDL_RotateSurface
fn run_rotate(t: &mut Harness) -> u64 {
    const RF: [usize; 7] = [6, 28, 24, 29, 20, 22, 31];
    const ANG: [f32; 12] = [
        0.0, 90.0, 180.0, 270.0, -90.0, 45.0, 30.0, 12.5, 359.0, 720.0, -33.0, 1.0,
    ];
    t.reset();
    for i in 0..7 {
        for (a, &angle) in ANG.iter().enumerate() {
            for v in 0..2 {
                let mut s = Surface::new(13, 9, FMTS[RF[i]]).unwrap();
                t.fill_random(&mut s);
                if s.palette.is_some() || v != 0 {
                    let key = if s.palette.is_some() {
                        t.rnd() % 256
                    } else {
                        first_pixel(&s)
                    };
                    s.set_color_key(Some(key)).unwrap();
                }
                if v != 0 {
                    s.set_blend_mode(BlendMode::MOD).unwrap();
                }
                if a == 3 {
                    s.properties()
                        .set(PROP_SURFACE_ROTATION_FLOAT, 10.4f32)
                        .unwrap();
                }
                let d = s.rotate(angle).ok();
                t.hash_surface(d.as_ref());
                if let Some(mut d) = d {
                    let key = d.color_key();
                    t.h8(key.is_some() as u8);
                    t.h32(key.unwrap_or(0));
                    t.h32(d.blend_mode().0);
                    let rot = d
                        .properties()
                        .get_float(PROP_SURFACE_ROTATION_FLOAT)
                        .unwrap_or(-1.0);
                    t.h32((rot * 100.0) as i32 as u32);
                    let clip = d.clip_rect();
                    t.h32(clip.w as u32);
                    t.h32(clip.h as u32);
                }
                t.dbg("rotate", i, a, v);
            }
        }
    }
    t.h
}

/// 8: YUV conversions
const YUVF: [PixelFormat; 11] = [
    PixelFormat::YV12,
    PixelFormat::IYUV,
    PixelFormat::YUY2,
    PixelFormat::UYVY,
    PixelFormat::YVYU,
    PixelFormat::NV12,
    PixelFormat::NV21,
    PixelFormat::P010,
    PixelFormat::I444,
    PixelFormat::I0FL,
    PixelFormat::I4FL,
];
const RGBF: [PixelFormat; 18] = [
    PixelFormat::RGB565,
    PixelFormat::BGR565,
    PixelFormat::RGB24,
    PixelFormat::BGR24,
    PixelFormat::XRGB8888,
    PixelFormat::RGBX8888,
    PixelFormat::XBGR8888,
    PixelFormat::BGRX8888,
    PixelFormat::ARGB8888,
    PixelFormat::RGBA8888,
    PixelFormat::ABGR8888,
    PixelFormat::BGRA8888,
    PixelFormat::ARGB4444,
    PixelFormat::XBGR2101010,
    PixelFormat::ARGB2101010,
    PixelFormat::RGB48,
    PixelFormat::RGBA64,
    PixelFormat::RGBA128_FLOAT,
];
const YCS: [Colorspace; 9] = [
    Colorspace::UNKNOWN,
    Colorspace::JPEG,
    Colorspace::BT601_LIMITED,
    Colorspace::BT601_FULL,
    Colorspace::BT709_LIMITED,
    Colorspace::BT709_FULL,
    Colorspace::BT2020_LIMITED,
    Colorspace::BT2020_FULL,
    Colorspace::SRGB,
];
const YSIZES: [(i32, i32); 11] = [
    (1, 1),
    (2, 2),
    (3, 3),
    (7, 5),
    (32, 2),
    (33, 3),
    (64, 4),
    (65, 5),
    (31, 7),
    (96, 3),
    (40, 1),
];

impl Harness {
    fn rnd_buf(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.rnd() as u8).collect()
    }

    fn fill_rgb(&mut self, p: &mut [u8], f: PixelFormat) {
        if f.pixel_type() == crate::video::pixels::PixelType::ArrayF32 {
            for i in 0..p.len() / 4 {
                let v = self.rnd_float();
                p[4 * i..4 * i + 4].copy_from_slice(&v.to_ne_bytes());
            }
        } else {
            for b in p.iter_mut() {
                *b = self.rnd() as u8;
            }
        }
    }

    fn hash_yuv_surface(&mut self, s: Option<&Surface<'_>>) {
        let Some(s) = s else {
            self.h8(0xEE);
            return;
        };
        self.h32(s.format.0);
        self.h32(s.w as u32);
        self.h32(s.h as u32);
        let Some(px) = s.pixels.bytes() else {
            self.h8(0xDD);
            return;
        };
        let (size, _) = calculate_yuv_size(s.format, s.w, s.h).unwrap();
        let px = px[..size].to_vec();
        self.hb(&px);
    }

    fn err(&self, tag: &str, r: &Result<()>) {
        if self.debug {
            if let Err(e) = r {
                println!("  {tag} {e}");
            }
        }
    }
}

fn run_yuv(t: &mut Harness) -> u64 {
    t.reset();
    for (fi, &f) in YUVF.iter().enumerate() {
        for (si, &(w, hh)) in YSIZES.iter().enumerate() {
            for (ri, &r) in RGBF.iter().enumerate() {
                let (ysize, ypitch) = calculate_yuv_size(f, w, hh).unwrap();
                let cs = YCS[t.rnd() as usize % YCS.len()];
                let yuv = t.rnd_buf(ysize);
                let rpitch = w * r.bytes_per_pixel() as i32;
                let mut rgb = vec![0u8; (rpitch * hh) as usize];
                let res = convert_pixels_and_colorspace(
                    w,
                    hh,
                    f,
                    cs,
                    None,
                    &yuv,
                    ypitch as i32,
                    r,
                    Colorspace::UNKNOWN,
                    None,
                    &mut rgb,
                    rpitch,
                );
                t.hok(&res);
                if res.is_ok() {
                    t.hb(&rgb);
                }
                t.err("E1", &res);
                t.dbg("yuv2rgb", fi * 100 + si, ri, res.is_ok() as usize);

                t.fill_rgb(&mut rgb, r);
                let mut out = vec![0u8; ysize];
                let res = convert_pixels_and_colorspace(
                    w,
                    hh,
                    r,
                    Colorspace::UNKNOWN,
                    None,
                    &rgb,
                    rpitch,
                    f,
                    cs,
                    None,
                    &mut out,
                    ypitch as i32,
                );
                t.hok(&res);
                if res.is_ok() {
                    t.hb(&out);
                }
                t.err("E2", &res);
                t.dbg("rgb2yuv", fi * 100 + si, ri, res.is_ok() as usize);
            }
        }
    }
    for (fi, &f) in YUVF.iter().enumerate() {
        for (gi, &g) in YUVF.iter().enumerate() {
            for (si, &(w, hh)) in YSIZES.iter().enumerate() {
                let (fsize, fpitch) = calculate_yuv_size(f, w, hh).unwrap();
                let (gsize, gpitch) = calculate_yuv_size(g, w, hh).unwrap();
                let cs = YCS[t.rnd() as usize % YCS.len()];
                let mut cs2 = cs;
                if t.rnd() % 8 == 0 {
                    cs2 = YCS[t.rnd() as usize % YCS.len()];
                }
                let src = t.rnd_buf(fsize);
                let mut dst = vec![0u8; gsize];
                let res = convert_pixels_and_colorspace(
                    w,
                    hh,
                    f,
                    cs,
                    None,
                    &src,
                    fpitch as i32,
                    g,
                    cs2,
                    None,
                    &mut dst,
                    gpitch as i32,
                );
                t.hok(&res);
                if res.is_ok() {
                    t.hb(&dst);
                }
                t.err("E3", &res);
                t.dbg("yuv2yuv", fi * 100 + gi, si, res.is_ok() as usize);
            }
        }
    }
    // surfaces
    for (fi, &f) in YUVF.iter().enumerate() {
        for (si, &(w, hh)) in YSIZES.iter().enumerate() {
            let mut s = Surface::new(w, hh, f).unwrap();
            let (size, _) = calculate_yuv_size(f, w, hh).unwrap();
            let bytes = t.rnd_buf(size);
            s.pixels.bytes_mut().unwrap()[..size].copy_from_slice(&bytes);
            let cs = YCS[1 + t.rnd() as usize % 5];
            s.set_colorspace(cs);
            let r = RGBF[t.rnd() as usize % RGBF.len()];
            let d = s.convert(r).ok();
            t.hash_surface(d.as_ref());
            let back = d
                .as_ref()
                .and_then(|d| d.convert_with_colorspace(f, None, cs, None).ok());
            t.hash_yuv_surface(back.as_ref());
            let w2 = 1 + (t.rnd() % 70) as i32;
            let h2 = 1 + (t.rnd() % 9) as i32;
            let mode = if t.rnd() & 1 != 0 {
                ScaleMode::Linear
            } else {
                ScaleMode::Nearest
            };
            let sc = s.scale(w2, h2, mode).ok();
            t.hash_yuv_surface(sc.as_ref());
            let dup = s.duplicate().ok();
            t.hash_yuv_surface(dup.as_ref());
            t.dbg("yuvsurf", fi, si, 0);
        }
    }
    t.h
}

/// Run `f` with upstream's x86 kernels selected (`simd`) or not.
fn with_simd(simd: bool, f: impl FnOnce(&mut Harness) -> u64) -> u64 {
    // For comparing per-case output with the C harness: run one mode only.
    if let Ok(only) = std::env::var("SDL_SURFACE_TEST_ONLY_SIMD") {
        if only != (simd as u8).to_string() {
            return 0;
        }
    }
    let s = crate::video::blit::SimdSupport {
        mmx: simd,
        sse: simd,
        sse2: simd,
        sse41: simd,
        avx2: simd,
    };
    crate::video::blit::SIMD_OVERRIDE.with(|o| o.set(Some(s)));
    let h = f(&mut Harness::new());
    crate::video::blit::SIMD_OVERRIDE.with(|o| o.set(None));
    h
}

// Hashes printed by the upstream C harness, with
// SDL_HasMMX/SSE2/SSE41/AVX2 reporting true and false.
#[test]
fn convert_matches_c() {
    let simd = with_simd(true, run_convert);
    let plain = with_simd(false, run_convert);
    assert_eq!((simd, plain), (0x11de2e0032a6ba2d, 0xeeef809f7bd0f921));
}

#[test]
fn blit_matches_c() {
    let simd = with_simd(true, run_blit);
    let plain = with_simd(false, run_blit);
    assert_eq!((simd, plain), (0x812674e3573cf57d, 0x3d7f5cfcb969acac));
}

#[test]
fn scaled_blit_matches_c() {
    let simd = with_simd(true, run_scaled);
    let plain = with_simd(false, run_scaled);
    assert_eq!((simd, plain), (0xe3d3a7ad6d866818, 0xd2c02d221a047f4e));
}

#[test]
fn misc_matches_c() {
    let simd = with_simd(true, run_misc);
    let plain = with_simd(false, run_misc);
    assert_eq!((simd, plain), (0x54c5089e1dcbc6e0, 0x90e771243b39fd3f));
}

#[test]
fn colorspace_blit_matches_c() {
    let simd = with_simd(true, run_colorspace);
    let plain = with_simd(false, run_colorspace);
    assert_eq!((simd, plain), (0x19547e3981883d7a, 0x19547e3981883d7a));
}

/// The blit function upstream's selection logic picks (as read from
/// SDL_blit*.c), with and without the x86 kernels.
#[test]
fn blitter_selection() {
    use crate::video::blit::MapBlit;
    #[allow(clippy::type_complexity)]
    let cases: &[(
        PixelFormat,
        PixelFormat,
        BlendMode,
        Option<u8>,
        bool,
        &str,
        &str,
    )] = &[
        // (src, dst, blend, alpha mod, colorkey, with SIMD, without)
        (
            PixelFormat::RGB565,
            PixelFormat::RGB565,
            BlendMode::BLEND,
            Some(100),
            false,
            "Blit565to565SurfaceAlphaMMX",
            "Blit565to565SurfaceAlpha",
        ),
        (
            PixelFormat::XRGB1555,
            PixelFormat::XRGB1555,
            BlendMode::BLEND,
            Some(100),
            false,
            "Blit555to555SurfaceAlphaMMX",
            "Blit555to555SurfaceAlpha",
        ),
        (
            PixelFormat::XRGB8888,
            PixelFormat::ARGB8888,
            BlendMode::BLEND,
            Some(100),
            false,
            "Blit888to888SurfaceAlphaSSE2",
            "BlitRGBtoRGBSurfaceAlpha",
        ),
        (
            PixelFormat::RGBX8888,
            PixelFormat::RGBX8888,
            BlendMode::BLEND,
            Some(100),
            false,
            "Blit888to888SurfaceAlphaSSE2",
            "BlitNtoNSurfaceAlpha",
        ),
        (
            PixelFormat::ARGB8888,
            PixelFormat::XRGB8888,
            BlendMode::NONE,
            None,
            false,
            "Blit8888to8888PixelSwizzleAVX2",
            "Blit4to4MaskAlpha",
        ),
        (
            PixelFormat::ARGB8888,
            PixelFormat::ABGR8888,
            BlendMode::NONE,
            None,
            false,
            "Blit8888to8888PixelSwizzleAVX2",
            "Blit_3or4_to_3or4__inversed_rgb",
        ),
        (
            PixelFormat::ARGB8888,
            PixelFormat::ARGB8888,
            BlendMode::BLEND,
            None,
            false,
            "Blit8888to8888PixelAlphaSwizzleAVX2",
            "Blit8888to8888PixelAlpha",
        ),
        (
            PixelFormat::ARGB8888,
            PixelFormat::ARGB8888,
            BlendMode::ADD,
            None,
            false,
            "SDL_Blit_ARGB8888_ARGB8888_Blend",
            "SDL_Blit_ARGB8888_ARGB8888_Blend",
        ),
        (
            PixelFormat::RGBA8888,
            PixelFormat::XBGR8888,
            BlendMode::MUL,
            Some(7),
            false,
            "SDL_Blit_RGBA8888_XBGR8888_Modulate_Blend",
            "SDL_Blit_RGBA8888_XBGR8888_Modulate_Blend",
        ),
        (
            PixelFormat::INDEX8,
            PixelFormat::ARGB8888,
            BlendMode::NONE,
            None,
            false,
            "Blit1to4",
            "Blit1to4",
        ),
        (
            PixelFormat::INDEX4LSB,
            PixelFormat::RGB565,
            BlendMode::NONE,
            None,
            true,
            "Blit4bto2Key",
            "Blit4bto2Key",
        ),
        (
            PixelFormat::RGB565,
            PixelFormat::ARGB8888,
            BlendMode::NONE,
            None,
            false,
            "Blit_RGB565_32_SSE41",
            "Blit_RGB565_ARGB8888",
        ),
        (
            PixelFormat::ARGB8888,
            PixelFormat::RGB565,
            BlendMode::BLEND,
            None,
            false,
            "BlitARGBto565PixelAlpha",
            "BlitARGBto565PixelAlpha",
        ),
        (
            PixelFormat::RGB24,
            PixelFormat::BGR24,
            BlendMode::NONE,
            None,
            true,
            "BlitNtoNKey",
            "BlitNtoNKey",
        ),
        (
            PixelFormat::ARGB2101010,
            PixelFormat::ARGB8888,
            BlendMode::NONE,
            None,
            false,
            // 10-bit formats default to HDR10, so the colorspaces differ
            "SDL_Blit_Slow_Float",
            "SDL_Blit_Slow_Float",
        ),
        (
            PixelFormat::ARGB2101010,
            PixelFormat::ABGR2101010,
            BlendMode::NONE,
            None,
            false,
            "SDL_Blit_Slow",
            "SDL_Blit_Slow",
        ),
        (
            PixelFormat::RGBA64,
            PixelFormat::ARGB8888,
            BlendMode::NONE,
            None,
            false,
            "SDL_Blit_Slow_Float",
            "SDL_Blit_Slow_Float",
        ),
    ];
    for &(sf, df, blend, amod, key, with, without) in cases {
        for (simd, expected) in [(true, with), (false, without)] {
            let s = crate::video::blit::SimdSupport {
                mmx: simd,
                sse: simd,
                sse2: simd,
                sse41: simd,
                avx2: simd,
            };
            crate::video::blit::SIMD_OVERRIDE.with(|o| o.set(Some(s)));
            let mut src = Surface::new(4, 4, sf).unwrap();
            let mut dst = Surface::new(4, 4, df).unwrap();
            if sf.is_indexed() {
                src.create_palette().unwrap();
            }
            src.set_blend_mode(blend).unwrap();
            if let Some(a) = amod {
                src.set_alpha_mod(a);
            }
            if key {
                src.set_color_key(Some(1)).unwrap();
            }
            src.blit(None, &mut dst, None).unwrap();
            let name = match src.map.blit {
                MapBlit::Soft(b) => b.name,
                MapBlit::Rle(_) => "rle",
                MapBlit::None => "none",
            };
            assert_eq!(name, expected, "{sf:?} -> {df:?} {blend:?} simd={simd}");
            crate::video::blit::SIMD_OVERRIDE.with(|o| o.set(None));
        }
    }
}

#[test]
fn rle_blit_matches_c() {
    let simd = with_simd(true, run_rle);
    let plain = with_simd(false, run_rle);
    assert_eq!((simd, plain), (0x8641058ac1efbb42, 0xea2b4ab8dd1e49c2));
}

#[test]
fn rotate_matches_c() {
    let simd = with_simd(true, run_rotate);
    let plain = with_simd(false, run_rotate);
    assert_eq!((simd, plain), (0x3460306225aab023, 0x541bb17330829833));
}

#[test]
fn yuv_matches_c() {
    let simd = with_simd(true, run_yuv);
    let plain = with_simd(false, run_yuv);
    assert_eq!((simd, plain), (0x0dbacdb8ec13c91b, 0x5a69c4ed3eec6936));
}
