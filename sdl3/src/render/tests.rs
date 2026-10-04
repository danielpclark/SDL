// Tests of the 2D renderer. `render_matches_c` replays the operations of a
// harness built from upstream's C sources (SDL_render.c, SDL_render_sw.c,
// SDL_yuv_sw.c and the surface code) and compares FNV-1a hashes of every
// result, as the surface tests do.

use super::*;
use crate::video::pixels::{Color, Palette};
use crate::video::surface::tests::{first_pixel, with_simd, Harness, FMTS, MODES};
use crate::video::surface::{read_palette, share_palette, write_palette};
use crate::video::FlipMode;

const RFMT: [usize; 17] = [
    0, 6, 7, 8, 10, 12, 16, 20, 21, 22, 24, 25, 26, 28, 29, 30, 31,
];
const TF: [usize; 12] = [4, 6, 7, 8, 12, 20, 22, 24, 26, 28, 30, 34];
const SF: [PixelFormat; 16] = [
    PixelFormat::INDEX8,
    PixelFormat::INDEX4LSB,
    PixelFormat::RGB565,
    PixelFormat::RGB24,
    PixelFormat::XRGB8888,
    PixelFormat::ARGB8888,
    PixelFormat::ABGR8888,
    PixelFormat::YV12,
    PixelFormat::IYUV,
    PixelFormat::NV12,
    PixelFormat::NV21,
    PixelFormat::YUY2,
    PixelFormat::UYVY,
    PixelFormat::YVYU,
    PixelFormat::RGB332,
    PixelFormat::ARGB2101010,
];
const TGT: [PixelFormat; 8] = [
    PixelFormat::UNKNOWN,
    PixelFormat::ARGB8888,
    PixelFormat::XRGB8888,
    PixelFormat::RGB565,
    PixelFormat::RGB24,
    PixelFormat::ABGR8888,
    PixelFormat::INDEX8,
    PixelFormat::YV12,
];
const DMODES: [BlendMode; 8] = [
    BlendMode::NONE,
    BlendMode::BLEND,
    BlendMode::BLEND_PREMULTIPLIED,
    BlendMode::ADD,
    BlendMode::ADD_PREMULTIPLIED,
    BlendMode::MOD,
    BlendMode::MUL,
    BlendMode(0x12345),
];
const ANG: [f64; 7] = [0.0, 90.0, 33.25, 180.0, -45.5, 720.0, 270.0];
const TSC: [f32; 5] = [0.5, 1.0, 1.5, 2.0, 0.0];
const GSC: [f32; 4] = [0.0, 1.0, 2.0, 0.5];
const SC: [f32; 4] = [1.0, 2.0, 0.5, 1.5];
const RAM: [TextureAddressMode; 3] = [
    TextureAddressMode::Auto,
    TextureAddressMode::Clamp,
    TextureAddressMode::Wrap,
];
const TEXTS: [&str; 5] = [
    "Hi!",
    "SDL3 \u{e9}",
    "~{}|",
    "\u{7f}\u{a0}x",
    "abcdefghijklmnop",
];
const FLIPS: [FlipMode; 4] = [
    FlipMode::None,
    FlipMode::Horizontal,
    FlipMode::Vertical,
    FlipMode::HorizontalAndVertical,
];
const SCALE_MODES: [ScaleMode; 3] = [ScaleMode::Nearest, ScaleMode::Linear, ScaleMode::PixelArt];
const LOGICAL: [LogicalPresentation; 5] = [
    LogicalPresentation::Disabled,
    LogicalPresentation::Stretch,
    LogicalPresentation::Letterbox,
    LogicalPresentation::Overscan,
    LogicalPresentation::IntegerScale,
];

fn rf(t: &mut Harness, n: i32) -> f32 {
    ((t.rnd() % (n * 4 + 48) as u32) as i32 - 24) as f32 / 4.0
}

fn rfrect(t: &mut Harness, w: i32, hh: i32) -> FRect {
    let x = rf(t, w);
    let y = rf(t, hh);
    let rw = rf(t, w);
    let rh = rf(t, hh);
    FRect { x, y, w: rw, h: rh }
}

fn rfcolor(t: &mut Harness) -> FColor {
    let r = (t.rnd() % 256) as f32 / 255.0;
    let g = (t.rnd() % 256) as f32 / 255.0;
    let b = (t.rnd() % 256) as f32 / 255.0;
    let a = (t.rnd() % 256) as f32 / 255.0;
    FColor { r, g, b, a }
}

fn fmt_row_bytes(f: PixelFormat, w: i32) -> usize {
    if f.is_fourcc() {
        (w * f.bytes_per_pixel() as i32) as usize
    } else {
        ((w * f.bits_per_pixel() as i32 + 7) / 8) as usize
    }
}

fn fill_rbuf(t: &mut Harness, rbuf: &mut [u8], n: usize) {
    for b in rbuf[..n].iter_mut() {
        *b = t.rnd() as u8;
    }
}

fn texture_format(r: &mut Renderer, t: Texture) -> PixelFormat {
    let props = r.texture_properties(t).unwrap();
    PixelFormat(props.get_number(PROP_TEXTURE_FORMAT_NUMBER).unwrap_or(0) as u32)
}

/// 11: the renderer front end with the software renderer
fn run_render(t: &mut Harness) -> u64 {
    t.reset();
    let mut rbuf = vec![0u8; 8192];
    for fi in 0..RFMT.len() {
        for k in 0..25 {
            let (w, hh) = (29, 21);
            let mut s = Surface::new(w, hh, FMTS[RFMT[fi]]).unwrap();
            t.fill_random(&mut s);
            t.dump = std::env::var("SDL_RENDER_TEST_DUMP").is_ok_and(|d| d == format!("{fi} {k}"));
            let r = Renderer::software(s);
            t.h8(r.is_ok() as u8);
            let Ok(mut r) = r else { continue };

            let mut tex: [Option<Texture>; 3] = [None; 3];
            {
                let tw = 1 + (t.rnd() % 12) as i32;
                let th = 1 + (t.rnd() % 10) as i32;
                let mut src =
                    Surface::new(tw, th, FMTS[TF[(t.rnd() % TF.len() as u32) as usize]]).unwrap();
                t.fill_random(&mut src);
                let v = t.rnd();
                if v & 1 != 0 {
                    let key = match src.palette() {
                        Some(p) => {
                            let n = read_palette(p).len() as u32;
                            t.rnd() % n
                        }
                        None => first_pixel(&src),
                    };
                    src.set_color_key(Some(key)).unwrap();
                }
                if v & 2 != 0 {
                    src.set_blend_mode(MODES[(t.rnd() % 7) as usize]).unwrap();
                }
                if v & 4 != 0 {
                    let a = t.rnd() as u8;
                    src.set_alpha_mod(a);
                }
                if v & 8 != 0 {
                    let (cr, cg, cb) = (t.rnd() as u8, t.rnd() as u8, t.rnd() as u8);
                    src.set_color_mod(cr, cg, cb);
                }
                tex[0] = r.create_texture_from_surface(&mut src).ok();
            }
            {
                let tw = 1 + (t.rnd() % 12) as i32;
                let th = 1 + (t.rnd() % 10) as i32;
                let f = SF[(t.rnd() % SF.len() as u32) as usize];
                tex[1] = r.create_texture(f, TextureAccess::Streaming, tw, th).ok();
                if f.is_indexed() {
                    let nc = 1usize << f.bits_per_pixel();
                    let p = share_palette(Palette::new(nc).unwrap());
                    for i in 0..nc {
                        let cr = t.rnd() as u8;
                        let cg = t.rnd() as u8;
                        let cb = t.rnd() as u8;
                        let ca = if t.rnd() & 1 != 0 { 255 } else { t.rnd() as u8 };
                        write_palette(&p)
                            .set_colors(i, &[Color::new(cr, cg, cb, ca)])
                            .unwrap();
                    }
                    if let Some(t1) = tex[1] {
                        let _ = r.set_texture_palette(t1, Some(p));
                    }
                }
                let pitch = 48 + (t.rnd() % 17) as i32;
                fill_rbuf(t, &mut rbuf, (pitch * 36 + 64) as usize);
                if let Some(t1) = tex[1] {
                    let ok = r.update_texture(t1, None, &rbuf, pitch);
                    t.hok(&ok);
                }
            }
            {
                let tw = 1 + (t.rnd() % 12) as i32;
                let th = 1 + (t.rnd() % 10) as i32;
                let f = TGT[(t.rnd() % TGT.len() as u32) as usize];
                tex[2] = r.create_texture(f, TextureAccess::Target, tw, th).ok();
            }
            for x in tex {
                t.h8(x.is_some() as u8);
            }
            t.dbg("setup", fi, k, 0);

            for _ in 0..16 {
                let op = t.rnd() % 26;
                let tt = tex[(t.rnd() % 3) as usize];
                let v = t.rnd();
                let ok: Result<()> = match op {
                    0 => {
                        let (cr, cg, cb, ca) =
                            (t.rnd() as u8, t.rnd() as u8, t.rnd() as u8, t.rnd() as u8);
                        r.set_draw_color(cr, cg, cb, ca);
                        Ok(())
                    }
                    1 => r.set_draw_blend_mode(DMODES[(t.rnd() % 8) as usize]),
                    2 => r.clear(),
                    3 | 4 => {
                        let n = 1 + (t.rnd() % 6) as usize;
                        let mut p = [FPoint::default(); 6];
                        for q in p.iter_mut() {
                            q.x = rf(t, w);
                            q.y = rf(t, hh);
                        }
                        if v & 1 != 0 {
                            p[n - 1] = p[0];
                        }
                        if op == 3 {
                            r.render_points(&p[..n])
                        } else {
                            r.render_lines(&p[..n])
                        }
                    }
                    5 | 6 => {
                        let n = (t.rnd() % 4) as usize;
                        let mut rr = [FRect::default(); 3];
                        for x in rr.iter_mut() {
                            *x = rfrect(t, w, hh);
                        }
                        match (op, n) {
                            (5, 0) => r.render_rect(None),
                            (5, _) => r.render_rects(&rr[..n]),
                            (_, 0) => r.render_fill_rect(None),
                            _ => r.render_fill_rects(&rr[..n]),
                        }
                    }
                    7..=10 | 23 => {
                        let a = rfrect(t, 12, 10);
                        let b = rfrect(t, w, hh);
                        let angle = ANG[(t.rnd() % 7) as usize];
                        let cx = rf(t, 12);
                        let cy = rf(t, 10);
                        let c = FPoint { x: cx, y: cy };
                        let flip = FLIPS[(t.rnd() % 4) as usize];
                        let scale = TSC[(t.rnd() % 5) as usize];
                        let gl = (t.rnd() % 5) as f32;
                        let gr = (t.rnd() % 5) as f32;
                        let gt = (t.rnd() % 5) as f32;
                        let gb = (t.rnd() % 5) as f32;
                        let gscale = GSC[(t.rnd() % 4) as usize];
                        let mut pts = [FPoint::default(); 3];
                        for q in pts.iter_mut() {
                            q.x = rf(t, w);
                            q.y = rf(t, hh);
                        }
                        let [o, rt, dn] = pts;
                        let pa = (v & 1 != 0).then_some(&a);
                        let pb = (v & 2 != 0).then_some(&b);
                        match tt {
                            None => Ok(()),
                            Some(tt) => match op {
                                7 => r.render_texture(tt, pa, pb),
                                8 => r.render_texture_rotated(
                                    tt,
                                    pa,
                                    pb,
                                    angle,
                                    (v & 4 != 0).then_some(&c),
                                    flip,
                                ),
                                9 => r.render_texture_tiled(tt, pa, scale, pb),
                                10 if v & 8 != 0 => r.render_texture_9grid_tiled(
                                    tt,
                                    pa,
                                    gl,
                                    gr,
                                    gt,
                                    gb,
                                    gscale,
                                    pb,
                                    if scale > 0.0 { scale } else { 1.0 },
                                ),
                                10 => r.render_texture_9grid(tt, pa, gl, gr, gt, gb, gscale, pb),
                                _ => r.render_texture_affine(
                                    tt,
                                    pa,
                                    (v & 4 != 0).then_some(&o),
                                    (v & 8 != 0).then_some(&rt),
                                    (v & 16 != 0).then_some(&dn),
                                ),
                            },
                        }
                    }
                    11 => {
                        let mut vx = [Vertex::default(); 6];
                        let idx = [0, 1, 2, 0, 2, 3];
                        let mut nv = 3 + 3 * (t.rnd() % 2) as usize;
                        for x in vx.iter_mut() {
                            x.position.x = rf(t, w);
                            x.position.y = rf(t, hh);
                            x.color = rfcolor(t);
                            x.tex_coord.x = (t.rnd() % 9) as f32 / 4.0 - 0.5;
                            x.tex_coord.y = (t.rnd() % 9) as f32 / 4.0 - 0.5;
                        }
                        if v & 4 != 0 {
                            // an axis aligned quad, for the rect path
                            let x0 = rf(t, w);
                            let y0 = rf(t, hh);
                            let x1 = rf(t, w);
                            let y1 = rf(t, hh);
                            let col = rfcolor(t);
                            let u0 = (t.rnd() % 5) as f32 / 4.0;
                            let u1 = (t.rnd() % 5) as f32 / 4.0;
                            let px = [x0, x1, x1, x0];
                            let py = [y0, y0, y1, y1];
                            let pu = [u0, u1, u1, u0];
                            let pv = [0.0, 0.0, 1.0, 1.0];
                            for i in 0..4 {
                                vx[i] = Vertex {
                                    position: FPoint { x: px[i], y: py[i] },
                                    color: col,
                                    tex_coord: FPoint { x: pu[i], y: pv[i] },
                                };
                            }
                            if v & 8 == 0 {
                                vx[4] = vx[2];
                                vx[5] = vx[3];
                                vx[3] = vx[0];
                                nv = 6;
                            }
                        }
                        let gt = if v & 1 != 0 { tt } else { None };
                        if v & 4 != 0 && v & 8 != 0 {
                            r.render_geometry(gt, &vx[..4], Some(&idx[..6]))
                        } else if v & 2 != 0 {
                            r.render_geometry(
                                gt,
                                &vx[..nv],
                                Some(&idx[..if nv == 6 { 6 } else { 3 }]),
                            )
                        } else {
                            r.render_geometry(gt, &vx[..nv], None)
                        }
                    }
                    12 | 13 => {
                        let x = (t.rnd() % 41) as i32 - 6;
                        let y = (t.rnd() % 33) as i32 - 6;
                        let rw = (t.rnd() % 32) as i32 - 2;
                        let rh = (t.rnd() % 24) as i32 - 2;
                        let rc = Rect::new(x, y, rw, rh);
                        let prc = if v & 1 != 0 { None } else { Some(&rc) };
                        if op == 12 {
                            r.set_viewport(prc)
                        } else {
                            r.set_clip_rect(prc)
                        }
                    }
                    14 => {
                        let sx = SC[(t.rnd() % 4) as usize];
                        let sy = SC[(t.rnd() % 4) as usize];
                        r.set_scale(sx, sy)
                    }
                    15 => {
                        let mode = LOGICAL[(t.rnd() % 5) as usize];
                        let lw = 1 + (t.rnd() % 40) as i32;
                        let lh = 1 + (t.rnd() % 30) as i32;
                        r.set_logical_presentation(lw, lh, mode)
                    }
                    16 => r.set_render_target(if v & 1 != 0 { None } else { tex[2] }),
                    17 => {
                        let (cr, cg, cb, ca) =
                            (t.rnd() as u8, t.rnd() as u8, t.rnd() as u8, t.rnd() as u8);
                        let bm = DMODES[(t.rnd() % 8) as usize];
                        let sm = SCALE_MODES[(t.rnd() % 3) as usize];
                        match tt {
                            None => Ok(()),
                            Some(tt) => {
                                let _ = r.set_texture_color_mod(tt, cr, cg, cb);
                                let _ = r.set_texture_alpha_mod(tt, ca);
                                let b = r.set_texture_blend_mode(tt, bm);
                                t.hok(&b);
                                r.set_texture_scale_mode(tt, sm)
                            }
                        }
                    }
                    18 => {
                        let x = rf(t, w);
                        let y = rf(t, hh);
                        r.render_debug_text(x, y, TEXTS[(t.rnd() % 5) as usize])
                    }
                    19 => {
                        let x = (t.rnd() % 12) as i32 - 2;
                        let y = (t.rnd() % 10) as i32 - 2;
                        let rw = (t.rnd() % 14) as i32;
                        let rh = (t.rnd() % 12) as i32;
                        let rc = Rect::new(x, y, rw, rh);
                        let pitch = 48 + (t.rnd() % 17) as i32;
                        fill_rbuf(t, &mut rbuf, (pitch * 36 + 64) as usize);
                        match tt {
                            None => Ok(()),
                            Some(tt) => {
                                let f = texture_format(&mut r, tt);
                                let full =
                                    v & 1 != 0 || f == PixelFormat::NV12 || f == PixelFormat::NV21;
                                r.update_texture(
                                    tt,
                                    if full { None } else { Some(&rc) },
                                    &rbuf,
                                    pitch,
                                )
                            }
                        }
                    }
                    20 => {
                        let x = (t.rnd() % 41) as i32 - 6;
                        let y = (t.rnd() % 33) as i32 - 6;
                        let rw = (t.rnd() % 30) as i32;
                        let rh = (t.rnd() % 20) as i32;
                        let rc = Rect::new(x, y, rw, rh);
                        let rp = r.read_pixels(if v & 1 != 0 { None } else { Some(&rc) });
                        let ok = rp.as_ref().map(|_| ()).map_err(|e| e.clone());
                        t.hash_surface(rp.as_ref().ok());
                        ok
                    }
                    21 | 25 => {
                        let (a, b, c, d) = (t.rnd(), t.rnd(), t.rnd(), t.rnd());
                        let color = t.rnd();
                        match tt {
                            None => Ok(()),
                            Some(tt) => {
                                let (fw, fh) = r.texture_size(tt).unwrap();
                                let (tw, th) = (fw as i32, fh as i32);
                                let mut rc =
                                    Rect::new((a % tw as u32) as i32, (b % th as u32) as i32, 0, 0);
                                rc.w = 1 + (c % (tw - rc.x) as u32) as i32;
                                rc.h = 1 + (d % (th - rc.y) as u32) as i32;
                                let whole = v & 1 != 0;
                                if whole {
                                    rc = Rect::new(0, 0, tw, th);
                                }
                                let prc = if whole { None } else { Some(&rc) };
                                let f = texture_format(&mut r, tt);
                                if op == 21 {
                                    match r.lock_texture(tt, prc) {
                                        Ok(mut lock) => {
                                            let rb = fmt_row_bytes(f, rc.w);
                                            let pitch = lock.pitch() as usize;
                                            for y in 0..rc.h as usize {
                                                fill_rbuf(t, &mut rbuf, rb);
                                                lock.pixels()[y * pitch..y * pitch + rb]
                                                    .copy_from_slice(&rbuf[..rb]);
                                            }
                                            Ok(())
                                        }
                                        Err(e) => Err(e),
                                    }
                                } else {
                                    match r.lock_texture_to_surface(tt, prc) {
                                        Ok(mut lock) => {
                                            let filled = lock
                                                .surface()
                                                .and_then(|mut s| s.fill_rect(None, color));
                                            t.hok(&filled);
                                            Ok(())
                                        }
                                        Err(e) => Err(e),
                                    }
                                }
                            }
                        }
                    }
                    22 => {
                        let idx = t.rnd();
                        let c =
                            Color::new(t.rnd() as u8, t.rnd() as u8, t.rnd() as u8, t.rnd() as u8);
                        match tt.and_then(|tt| r.texture_palette(tt).ok().flatten()) {
                            Some(p) => {
                                let mut p = write_palette(&p);
                                let n = p.len() as u32;
                                p.set_colors((idx % n) as usize, &[c])
                            }
                            None => Ok(()),
                        }
                    }
                    24 => {
                        let mu = RAM[(t.rnd() % 3) as usize];
                        let mv = RAM[(t.rnd() % 3) as usize];
                        r.set_texture_address_mode(mu, mv);
                        Ok(())
                    }
                    _ => unreachable!(),
                };
                t.hok(&ok);
                t.dbg("op", fi, k, op as usize);
            }

            let rp = r.read_pixels(None);
            t.hok(&rp);
            t.hash_surface(rp.as_ref().ok());
            let presented = r.present();
            t.hok(&presented);
            let s = r.into_surface();
            t.hash_surface(s.as_ref());
            t.dbg("end", fi, k, 0);
        }
    }
    t.h
}

// The hash printed by the upstream C harness (with SDL_HasMMX/SSE2/SSE41/AVX2
// reporting true and false). The INDEX8 output (FMTS[6]) needs upstream's
// SDL_render_sw.c patched to give the intermediate surface of a scaled,
// clipped copy the output's palette, as `run_commands` does; upstream
// crashes there.
#[test]
fn render_matches_c() {
    let simd = with_simd(true, run_render);
    let plain = with_simd(false, run_render);
    assert_eq!((simd, plain), (0x92f63792ff6bb6d0, 0xea36e230679f5eb7));
}

#[test]
fn renderer_basics() {
    let s = Surface::new(8, 8, PixelFormat::ARGB8888).unwrap();
    let mut r = Renderer::software(s).unwrap();
    assert_eq!(r.name(), SOFTWARE_RENDERER);
    assert_eq!(r.output_size().unwrap(), (8, 8));
    r.set_draw_color(255, 0, 0, 255);
    r.clear().unwrap();
    let px = r.read_pixels(Some(&Rect::new(1, 1, 1, 1))).unwrap();
    assert_eq!(px.read_pixel(0, 0).unwrap(), Color::new(255, 0, 0, 255));

    // Textures of a destroyed handle or another renderer are rejected
    let t = r
        .create_texture(PixelFormat::ARGB8888, TextureAccess::Streaming, 2, 2)
        .unwrap();
    r.destroy_texture(t);
    assert!(r.render_texture(t, None, None).is_err());
    let mut other = Renderer::software(Surface::new(2, 2, PixelFormat::ARGB8888).unwrap()).unwrap();
    let t2 = other
        .create_texture(PixelFormat::ARGB8888, TextureAccess::Static, 1, 1)
        .unwrap();
    assert_eq!(
        r.render_texture(t2, None, None).unwrap_err().to_string(),
        "Texture was not created with this renderer"
    );

    // A locked streaming texture is uploaded when the lock is dropped
    let t = r
        .create_texture(PixelFormat::RGB24, TextureAccess::Streaming, 2, 2)
        .unwrap();
    {
        let mut lock = r.lock_texture(t, None).unwrap();
        let pitch = lock.pitch() as usize;
        for y in 0..2 {
            lock.pixels()[y * pitch..y * pitch + 6].copy_from_slice(&[0, 255, 0, 0, 255, 0]);
        }
    }
    r.set_draw_blend_mode(BlendMode::NONE).unwrap();
    r.render_texture(t, None, None).unwrap();
    let px = r.read_pixels(None).unwrap();
    assert_eq!(px.read_pixel(7, 7).unwrap(), Color::new(0, 255, 0, 255));
    assert!(r.into_surface().is_some());
}

// Conversions of pixels with any pitch, as SDL_UpdateTexture() passes
// them (see BlitInfo::truncate_skips()).
#[test]
fn convert_pixels_any_pitch_matches_c() {
    fn run(t: &mut Harness) -> u64 {
        use crate::video::surface::convert_pixels;
        const PF: [PixelFormat; 6] = [
            PixelFormat::RGB565,
            PixelFormat::XBGR8888,
            PixelFormat::ABGR8888,
            PixelFormat::XRGB8888,
            PixelFormat::ARGB8888,
            PixelFormat::BGR565,
        ];
        t.reset();
        let mut rbuf = vec![0u8; 8192];
        for k in 0..400 {
            let w = 1 + (t.rnd() % 12) as i32;
            let hh = 1 + (t.rnd() % 10) as i32;
            let pitch = 48 + (t.rnd() % 17) as i32;
            let sf = PF[(t.rnd() % 6) as usize];
            let df = PF[(t.rnd() % 6) as usize];
            fill_rbuf(t, &mut rbuf, (pitch * 36 + 64) as usize);
            let mut out = vec![0u8; 4096];
            let dpitch = w * 4 + 4 * (t.rnd() % 3) as i32;
            let ok = convert_pixels(w, hh, sf, &rbuf, pitch, df, &mut out, dpitch);
            t.hok(&ok);
            t.hb(&out[..(dpitch * hh) as usize]);
            t.dbg("pitch", k, w as usize, hh as usize);
        }
        t.h
    }
    let simd = with_simd(true, run);
    let plain = with_simd(false, run);
    assert_eq!((simd, plain), (0x34882d8a52ffb322, 0x621e5cedca99fdce));
}

// A session with window renderers on the dummy video driver, compared with
// the same session run by upstream's C (testdata/window_renderer_trace.txt,
// from a program built against SDL with only the dummy and offscreen video
// drivers).
mod window_session {
    use std::fmt::Write as _;

    use crate::events::window::WindowFlags;
    use crate::events::{
        get_events, pump, DropEvent, Event, EventType, MouseButtonEvent, MouseMotionEvent,
        MouseWheelEvent, TouchFingerEvent,
    };
    use crate::hints;
    use crate::init::{self, InitFlags};
    use crate::render::*;
    use crate::test_support::TEST_LOCK;
    use crate::video::rect::{FRect, Rect};
    use crate::video::{PixelFormat, Surface, Window};

    /// `%g`.
    fn g(v: f32) -> String {
        let v = v as f64;
        if v == 0.0 {
            return "0".into();
        }
        let exp = v.abs().log10().floor() as i32;
        let decimals = (5 - exp).max(0) as usize;
        let s = format!("{v:.decimals$}");
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    }

    fn res<T>(out: &mut String, label: &str, r: crate::error::Result<T>) {
        match r {
            Ok(_) => writeln!(out, "{label}: ok").unwrap(),
            Err(e) => writeln!(out, "{label}: err: {}", e.message()).unwrap(),
        }
    }

    /// FNV-1a of the pixel rows.
    fn hash(s: &Surface<'_>) -> u32 {
        let mut h = 2166136261u32;
        let pixels = s.raw_pixels().unwrap();
        let row = (s.width() * s.format().bytes_per_pixel() as i32) as usize;
        for y in 0..s.height() as usize {
            for &b in &pixels[y * s.pitch() as usize..][..row] {
                h = (h ^ b as u32).wrapping_mul(16777619);
            }
        }
        h
    }

    fn readpix(out: &mut String, r: &mut Renderer, label: &str) {
        let s = match r.read_pixels(None) {
            Ok(s) => s,
            Err(e) => {
                writeln!(out, "{label}: err: {}", e.message()).unwrap();
                return;
            }
        };
        let p = s.read_pixel(0, 0).unwrap();
        let m = s.read_pixel(s.width() / 2, s.height() / 2).unwrap();
        writeln!(
            out,
            "{label}: {}x{} fmt={} hash={:08x} p00={},{},{},{} pmid={},{},{},{}",
            s.width(),
            s.height(),
            s.format().name(),
            hash(&s),
            p.r,
            p.g,
            p.b,
            p.a,
            m.r,
            m.g,
            m.b,
            m.a
        )
        .unwrap();
    }

    fn sizes(out: &mut String, r: &mut Renderer, label: &str) {
        let (w, h) = match r.output_size() {
            Ok(s) => s,
            Err(e) => {
                writeln!(out, "{label}: err: {}", e.message()).unwrap();
                return;
            }
        };
        let (cw, ch) = r.current_output_size();
        let vp = r.viewport();
        let (sok, safe) = match r.safe_area() {
            Ok(s) => (1, s),
            Err(_) => (0, Rect::default()),
        };
        writeln!(
            out,
            "{label}: out={w}x{h} cur={cw}x{ch}(1) vp={},{},{},{} safe={sok}:{},{},{},{}",
            vp.x, vp.y, vp.w, vp.h, safe.x, safe.y, safe.w, safe.h
        )
        .unwrap();
    }

    fn drain() {
        pump();
        let _ = get_events(EventType::FIRST, EventType::LAST, 10000);
    }

    fn rect(out: &mut String, label: &str, r: FRect) {
        writeln!(out, "{label}={},{},{},{}", g(r.x), g(r.y), g(r.w), g(r.h)).unwrap();
    }

    fn nil(w: Option<Window>) -> &'static str {
        match w {
            Some(_) => "window",
            None => "(nil)",
        }
    }

    fn session() -> String {
        let mut out = String::new();
        let o = &mut out;
        hints::set(hints::VIDEO_DRIVER, "dummy").unwrap();
        res(o, "init", init::init(InitFlags::VIDEO));
        drain();

        let created = create_window_and_renderer("wr", 64, 48, WindowFlags::default());
        res(
            o,
            "create",
            created.as_ref().map(|_| ()).map_err(Clone::clone),
        );
        let (win, mut r) = created.unwrap();
        let props = r.properties();
        writeln!(
            o,
            "name={} vsync=0 hidden={} hassurface={} samewin={} propwin={}",
            r.name(),
            win.flags().unwrap().contains(WindowFlags::HIDDEN) as i32,
            win.has_surface().unwrap() as i32,
            (r.window() == Some(win)) as i32,
            props
                .get_any::<Window>(PROP_RENDERER_WINDOW_POINTER)
                .is_some_and(|w| *w == win) as i32
        )
        .unwrap();
        writeln!(
            o,
            "vsync={} hdr={} white={} headroom={}",
            r.vsync(),
            props
                .get_bool(PROP_RENDERER_HDR_ENABLED_BOOLEAN)
                .unwrap_or(true) as i32,
            g(props
                .get_float(PROP_RENDERER_SDR_WHITE_POINT_FLOAT)
                .unwrap_or(-1.0)),
            g(props
                .get_float(PROP_RENDERER_HDR_HEADROOM_FLOAT)
                .unwrap_or(-1.0))
        )
        .unwrap();
        match Renderer::for_window(&win, None) {
            Ok(_) => writeln!(o, "second: ok").unwrap(),
            Err(e) => writeln!(o, "second: {}", e.message()).unwrap(),
        }
        sizes(o, &mut r, "sizes");
        drain();

        r.set_draw_color(10, 20, 30, 255);
        res(o, "clear", r.clear());
        r.set_draw_color(200, 100, 50, 255);
        res(
            o,
            "fill",
            r.render_fill_rect(Some(&FRect::new(8.0, 8.0, 32.0, 24.0))),
        );
        readpix(o, &mut r, "read1");
        res(o, "present", r.present());

        res(o, "resize", win.set_size(100, 80));
        sizes(o, &mut r, "resized");
        readpix(o, &mut r, "read2");
        drain();

        res(
            o,
            "logical",
            r.set_logical_presentation(32, 24, LogicalPresentation::Letterbox),
        );
        rect(o, "logical rect", r.logical_presentation_rect());
        sizes(o, &mut r, "logical sizes");
        {
            let id = win.id();
            let mut e = Event::MouseMotion(MouseMotionEvent {
                window_id: id,
                x: 50.0,
                y: 40.0,
                xrel: 10.0,
                yrel: 5.0,
                ..Default::default()
            });
            res(
                o,
                "convert motion",
                r.convert_event_to_render_coordinates(&mut e),
            );
            let Event::MouseMotion(m) = e else {
                unreachable!()
            };
            writeln!(
                o,
                "motion={},{} rel={},{}",
                g(m.x),
                g(m.y),
                g(m.xrel),
                g(m.yrel)
            )
            .unwrap();

            let mut e = Event::MouseMotion(MouseMotionEvent {
                window_id: 12345,
                x: 50.0,
                y: 40.0,
                ..Default::default()
            });
            r.convert_event_to_render_coordinates(&mut e).unwrap();
            let Event::MouseMotion(m) = e else {
                unreachable!()
            };
            writeln!(o, "other motion={},{}", g(m.x), g(m.y)).unwrap();

            let mut e = Event::MouseButton(MouseButtonEvent {
                window_id: id,
                x: 10.0,
                y: 70.0,
                down: true,
                ..Default::default()
            });
            r.convert_event_to_render_coordinates(&mut e).unwrap();
            let Event::MouseButton(b) = e else {
                unreachable!()
            };
            writeln!(o, "button={},{}", g(b.x), g(b.y)).unwrap();

            let mut e = Event::MouseWheel(MouseWheelEvent {
                window_id: id,
                mouse_x: 75.0,
                mouse_y: 20.0,
                x: 1.0,
                ..Default::default()
            });
            r.convert_event_to_render_coordinates(&mut e).unwrap();
            let Event::MouseWheel(w) = e else {
                unreachable!()
            };
            writeln!(o, "wheel={},{} {}", g(w.mouse_x), g(w.mouse_y), g(w.x)).unwrap();

            let mut e = Event::TouchFinger(TouchFingerEvent {
                event_type: EventType::FINGER_MOTION,
                x: 0.5,
                y: 0.25,
                dx: 0.1,
                dy: 0.2,
                ..Default::default()
            });
            r.convert_event_to_render_coordinates(&mut e).unwrap();
            let Event::TouchFinger(f) = e else {
                unreachable!()
            };
            writeln!(o, "finger={},{} d={},{}", g(f.x), g(f.y), g(f.dx), g(f.dy)).unwrap();

            let mut e = Event::Drop(DropEvent {
                event_type: EventType::DROP_POSITION,
                window_id: id,
                x: 30.0,
                y: 60.0,
                ..Default::default()
            });
            r.convert_event_to_render_coordinates(&mut e).unwrap();
            let Event::Drop(d) = e else { unreachable!() };
            writeln!(o, "drop={},{}", g(d.x), g(d.y)).unwrap();

            let (wx, wy) = r.coordinates_to_window(16.0, 12.0);
            writeln!(o, "to window={},{}", g(wx), g(wy)).unwrap();
        }
        r.set_draw_color(0, 0, 0, 255);
        r.clear().unwrap();
        r.set_draw_color(255, 255, 255, 255);
        r.render_fill_rect(Some(&FRect::new(0.0, 0.0, 16.0, 12.0)))
            .unwrap();
        readpix(o, &mut r, "read3");

        // Resize with a render target set: only the main view follows the window
        let t = r
            .create_texture(PixelFormat::ARGB8888, TextureAccess::Target, 20, 10)
            .unwrap();
        res(o, "target", r.set_render_target(Some(t)));
        sizes(o, &mut r, "target sizes");
        res(o, "resize2", win.set_size(120, 90));
        sizes(o, &mut r, "target resized");
        rect(o, "target logical rect", r.logical_presentation_rect());
        res(o, "untarget", r.set_render_target(None));
        sizes(o, &mut r, "untarget sizes");
        rect(o, "main logical rect", r.logical_presentation_rect());
        r.set_render_target(Some(t)).unwrap();
        res(o, "present on target", r.present());
        r.set_render_target(None).unwrap();
        res(
            o,
            "logical off",
            r.set_logical_presentation(0, 0, LogicalPresentation::Disabled),
        );
        drain();

        res(o, "hide", win.hide());
        res(o, "show", win.show());
        res(o, "minimize", win.minimize());
        res(o, "restore", win.restore());
        res(o, "present2", r.present());
        drain();

        // The window goes first
        win.destroy();
        res(o, "clear after", r.clear());
        sizes(o, &mut r, "sizes after");
        res(o, "present after", r.present());
        writeln!(o, "window after={}", nil(r.window())).unwrap();
        drop(r);
        drain();

        // Driver selection
        let w2 = Window::create("w2", 40, 30, WindowFlags::default()).unwrap();
        match Renderer::for_window(&w2, Some("nonexistent")) {
            Ok(_) => writeln!(o, "nonexistent: ok").unwrap(),
            Err(e) => writeln!(o, "nonexistent: {}", e.message()).unwrap(),
        }
        match Renderer::for_window(&w2, Some("nonexistent,SOFTWARE")) {
            Ok(r2) => writeln!(o, "list: {}", r2.name()).unwrap(),
            Err(e) => writeln!(o, "list: {}", e.message()).unwrap(),
        }
        writeln!(
            o,
            "surface after destroy={}",
            w2.has_surface().unwrap() as i32
        )
        .unwrap();
        hints::set(hints::RENDER_DRIVER, "bogus").unwrap();
        match Renderer::for_window(&w2, None) {
            Ok(_) => writeln!(o, "hint bogus: ok").unwrap(),
            Err(e) => writeln!(o, "hint bogus: {}", e.message()).unwrap(),
        }
        hints::set(hints::RENDER_DRIVER, "software").unwrap();
        let r2 = Renderer::for_window(&w2, None);
        match &r2 {
            Ok(r2) => writeln!(o, "hint software: {}", r2.name()).unwrap(),
            Err(e) => writeln!(o, "hint software: {}", e.message()).unwrap(),
        }
        hints::reset(hints::RENDER_DRIVER);
        writeln!(
            o,
            "window surface with renderer={}",
            w2.surface().is_ok() as i32
        )
        .unwrap();
        // The renderer goes first
        drop(r2);
        writeln!(
            o,
            "surface after destroy2={}",
            w2.has_surface().unwrap() as i32
        )
        .unwrap();
        {
            let sr =
                Renderer::software(Surface::new(4, 4, PixelFormat::ARGB8888).unwrap()).unwrap();
            writeln!(o, "sw window={}", nil(sr.window())).unwrap();
        }
        {
            let ws2 = w2.surface();
            writeln!(o, "got surface={}", ws2.is_ok() as i32).unwrap();
            match Renderer::for_window(&w2, None) {
                Ok(_) => writeln!(o, "with surface: ok").unwrap(),
                Err(e) => writeln!(o, "with surface: {}", e.message()).unwrap(),
            }
            w2.destroy_surface().unwrap();
        }
        w2.destroy();
        drain();

        // A transparent window with a shape
        let created = create_window_and_renderer(
            "w3",
            16,
            16,
            WindowFlags::TRANSPARENT | WindowFlags::HIDDEN,
        );
        res(
            o,
            "create transparent",
            created.as_ref().map(|_| ()).map_err(Clone::clone),
        );
        let (w3, mut r3) = created.unwrap();
        writeln!(
            o,
            "hidden={}",
            w3.flags().unwrap().contains(WindowFlags::HIDDEN) as i32
        )
        .unwrap();
        {
            let mut shape = Surface::new(16, 16, PixelFormat::ARGB8888).unwrap();
            shape.fill_rect(None, 0x00000000).unwrap();
            shape
                .fill_rect(Some(&Rect::new(4, 4, 8, 8)), 0xff000000)
                .unwrap();
            res(o, "shape", w3.set_shape(Some(&shape)));
        }
        r3.set_draw_color(255, 255, 255, 255);
        r3.clear().unwrap();
        res(o, "present shaped", r3.present());
        readpix(o, &mut r3, "shaped");
        r3.set_draw_color(255, 0, 0, 255);
        r3.clear().unwrap();
        res(o, "present shaped2", r3.present());
        readpix(o, &mut r3, "shaped2");
        res(o, "unshape", w3.set_shape(None));
        r3.clear().unwrap();
        res(o, "present unshaped", r3.present());
        readpix(o, &mut r3, "unshaped");
        drop(r3);
        w3.destroy();

        // Quitting video with a renderer alive
        let created = create_window_and_renderer("w4", 8, 8, WindowFlags::default());
        res(
            o,
            "create4",
            created.as_ref().map(|_| ()).map_err(Clone::clone),
        );
        let (_w4, mut r4) = created.unwrap();
        init::quit_subsystem(InitFlags::VIDEO);
        res(o, "clear after quit", r4.clear());
        drop(r4);
        init::quit();
        out
    }

    #[test]
    fn window_renderer_matches_c() {
        let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        init::quit(); // (in case an earlier test failed halfway)
        let out = session();
        hints::reset(hints::VIDEO_DRIVER);
        let expected = include_str!("testdata/window_renderer_trace.txt");
        if out != expected {
            let path = std::env::temp_dir().join("rust_window_renderer_trace.txt");
            std::fs::write(&path, &out).unwrap();
            panic!("trace differs from upstream's; see {}", path.display());
        }
    }
}
