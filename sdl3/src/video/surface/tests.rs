// Tests for the surface module. The `*_matches_c` tests replay the same
// operations as a harness built from upstream's C sources and compare
// FNV-1a hashes of every result, so any difference in any pixel, error or
// attribute changes the hash. The C side is configured like this module:
// without SDL_blit_0/1/A/N/auto, RLE and YUV (an upstream "lean" build),
// with allocations zeroed like Rust's.

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
const SUB: [usize; 18] = [
    6, 7, 8, 12, 16, 20, 22, 23, 24, 28, 29, 30, 31, 33, 34, 38, 42, 50,
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
                    let mut s = Surface::new(9, 7, FMTS[SUB[i]]).unwrap();
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
        let c = t.rnd();
        let ok = s.fill_rect(None, c);
        t.hok(&ok);
        t.hash_surface(Some(&s));
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

// Hashes printed by the upstream C harness (lean configuration).
#[test]
fn convert_matches_c() {
    assert_eq!(run_convert(&mut Harness::new()), 0x61e32c282566f407);
}

#[test]
fn blit_matches_c() {
    assert_eq!(run_blit(&mut Harness::new()), 0xf360e622b7c8e9ef);
}

#[test]
fn scaled_blit_matches_c() {
    assert_eq!(run_scaled(&mut Harness::new()), 0xdb883930588f2a01);
}

#[test]
fn misc_matches_c() {
    assert_eq!(run_misc(&mut Harness::new()), 0xe1bd252c3edefb02);
}

#[test]
fn colorspace_blit_matches_c() {
    assert_eq!(run_colorspace(&mut Harness::new()), 0xe708ec1d52fb29b3);
}
