// Tests of the image codecs: a session replayed from a C harness built
// against upstream SDL (its stb_image and miniz), whose output is
// testdata/image_trace.txt. The images in testdata/images come from
// tools/gen_png_testdata.py (PNG) and ImageMagick (JPEG).

use std::fmt::Write as _;

use crate::io::{IoStream, IoWhence};
use crate::video::pixels::{Color, PixelFormat};
use crate::video::surface::{read_palette, write_palette, Surface};

const FNV0: u64 = 14695981039346656037;

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

struct Lcg(u64);

impl Lcg {
    fn rnd(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
}

fn surf_hash(s: &Surface<'_>) -> u64 {
    let f = s.format();
    let row = if f.bits_per_pixel() < 8 {
        (s.width() as usize * f.bits_per_pixel() as usize).div_ceil(8)
    } else {
        s.width() as usize * f.bytes_per_pixel() as usize
    };
    let pixels = s.raw_pixels().unwrap();
    let mut h = FNV0;
    for y in 0..s.height() as usize {
        h = fnv(h, &pixels[y * s.pitch() as usize..][..row]);
    }
    h
}

fn describe(out: &mut String, label: &str, s: Result<&Surface<'_>, &crate::error::Error>) {
    let s = match s {
        Ok(s) => s,
        Err(e) => {
            writeln!(out, "{label}: err: {}", e.message()).unwrap();
            return;
        }
    };
    write!(
        out,
        "{label}: {}x{} {} pitch={} hash={:016x}",
        s.width(),
        s.height(),
        s.format().name(),
        s.pitch(),
        surf_hash(s)
    )
    .unwrap();
    if let Some(p) = s.palette() {
        let p = read_palette(p);
        let bytes: Vec<u8> = p
            .colors()
            .iter()
            .flat_map(|c| [c.r, c.g, c.b, c.a])
            .collect();
        write!(out, " pal={}:{:016x}", p.len(), fnv(FNV0, &bytes)).unwrap();
    }
    if let Some(key) = s.color_key() {
        write!(out, " key={key}").unwrap();
    }
    writeln!(out, " blend={}", s.blend_mode().0).unwrap();
}

fn save(out: &mut String, label: &str, s: &mut Surface<'_>) {
    let mut io = IoStream::from_dynamic_mem();
    if let Err(e) = s.save_png_io(&mut io) {
        writeln!(out, "{label}: save err: {}", e.message()).unwrap();
        return;
    }
    let n = io.size().unwrap();
    io.seek(0, IoWhence::Set).unwrap();
    let mut bytes = vec![0u8; n as usize];
    assert_eq!(io.read(&mut bytes), n as usize);
    writeln!(
        out,
        "{label}: saved {n} bytes hash={:016x}",
        fnv(FNV0, &bytes)
    )
    .unwrap();
    io.seek(0, IoWhence::Set).unwrap();
    let back = Surface::load_png_io(&mut io);
    describe(out, &format!("{label} reload"), back.as_ref());
}

fn mjpg(out: &mut String, name: &str, data: &[u8], w: i32, h: i32) {
    // (YUY2 is left out: upstream overflows its NV12 temporary buffer)
    let fmts = [
        PixelFormat::NV12,
        PixelFormat::YV12,
        PixelFormat::NV21,
        PixelFormat::P010,
        PixelFormat::RGBA32,
        PixelFormat::XRGB8888,
    ];
    for f in fmts {
        let (pitch, size) = if f.is_fourcc() {
            let bpp = if f == PixelFormat::P010 { 2 } else { 1 };
            let pitch = w * bpp;
            let size = pitch as usize * h as usize
                + ((w + 1) / 2) as usize * 2 * bpp as usize * ((h + 1) / 2) as usize;
            (pitch, size)
        } else {
            (w * 4, (w * 4) as usize * h as usize)
        };
        let mut dst = vec![0u8; size + 64];
        match crate::video::surface::convert_pixels(
            w,
            h,
            PixelFormat::MJPG,
            data,
            data.len() as i32,
            f,
            &mut dst,
            pitch,
        ) {
            Ok(()) => writeln!(
                out,
                "{name} mjpg {}: {:016x}",
                f.name(),
                fnv(FNV0, &dst[..size])
            )
            .unwrap(),
            Err(e) => writeln!(out, "{name} mjpg {}: err: {}", f.name(), e.message()).unwrap(),
        }
    }
}

fn session() -> String {
    let mut out = String::new();
    let o = &mut out;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/video/testdata/images");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    for name in &names {
        let path = format!("{dir}/{name}");
        let mut io = IoStream::from_file(&path, "rb").unwrap();
        let is_jpg = name.contains(".jpg");
        let mut s = if is_jpg {
            Surface::load_jpg_io(&mut io)
        } else {
            Surface::load_png_io(&mut io)
        };
        describe(o, name, s.as_ref());
        writeln!(o, "{name} pos={}", io.tell().unwrap()).unwrap();
        drop(io);
        if let Ok(s) = &mut s {
            save(o, &format!("{name} save"), s);
        }
        // (4:1:1 is left out: upstream's NV12 output reads outside the chroma
        // planes; progressive files too: with the NV12 stride bug, upstream
        // reads component padding that progressive decoding never writes)
        if let Ok(s) = &s {
            if is_jpg && !name.contains("411") && !name.contains("prog") {
                let data = std::fs::read(&path).unwrap();
                mjpg(o, name, &data, s.width(), s.height());
            }
        }
    }

    // Synthetic surfaces for the compressor
    let mut rng = Lcg(1);
    let cases = [
        ("random rgba", 61, 47, PixelFormat::RGBA32, 0),
        ("gradient argb", 300, 211, PixelFormat::ARGB8888, 1),
        ("stripes rgb565", 257, 130, PixelFormat::RGB565, 2),
        ("random index8", 123, 77, PixelFormat::INDEX8, 0),
        ("pattern index4", 50, 33, PixelFormat::INDEX4MSB, 2),
        ("pattern index1", 64, 20, PixelFormat::INDEX1LSB, 2),
        ("large mixed", 700, 500, PixelFormat::RGBA32, 3),
    ];
    for (label, w, h, f, mode) in cases {
        let mut s = Surface::new(w, h, f).unwrap();
        if f.is_indexed() {
            let p = s.create_palette().unwrap();
            let n = read_palette(&p).len();
            for k in 0..n {
                let (r, g, b) = (rng.rnd() as u8, rng.rnd() as u8, rng.rnd() as u8);
                let a = if k % 3 == 0 { 128 } else { 255 };
                write_palette(&p)
                    .set_colors(k, &[Color::new(r, g, b, a)])
                    .unwrap();
            }
        }
        let pitch = s.pitch() as usize;
        let px = s.pixels_mut().unwrap();
        for y in 0..h as usize {
            for x in 0..pitch {
                px[y * pitch + x] = match mode {
                    0 => rng.rnd() as u8,
                    1 => (x + y) as u8,
                    2 => ((x / 7) ^ (y / 3)).wrapping_mul(37) as u8,
                    _ => {
                        if ((x / 40 + y / 30) & 1) != 0 {
                            rng.rnd() as u8
                        } else {
                            (x * 3 + y) as u8
                        }
                    }
                };
            }
        }
        describe(o, label, Ok(&s));
        save(o, label, &mut s);
    }
    let mut nopal = Surface::new(4, 4, PixelFormat::INDEX8).unwrap();
    nopal.set_palette(None).unwrap();
    save(o, "no palette", &mut nopal);
    // Detection
    let notimg = b"hello world, this is not an image at all";
    let mut io = IoStream::from_const_mem(notimg);
    describe(o, "text as png", Surface::load_png_io(&mut io).as_ref());
    describe(o, "text as jpg", Surface::load_jpg_io(&mut io).as_ref());
    describe(o, "text as surface", Surface::load_io(&mut io).as_ref());
    writeln!(o, "text pos={}", io.tell().unwrap()).unwrap();
    out
}

#[test]
fn images_match_c() {
    let out = session();
    let expected = include_str!("../testdata/image_trace.txt");
    if out != expected {
        let path = std::env::temp_dir().join("rust_image_trace.txt");
        std::fs::write(&path, &out).unwrap();
        for (i, (a, b)) in out.lines().zip(expected.lines()).enumerate() {
            assert_eq!(a, b, "line {} (full trace in {})", i + 1, path.display());
        }
        assert_eq!(out.lines().count(), expected.lines().count());
    }
}

/// The conversions upstream can't run safely: MJPG to YUY2 (upstream
/// overflows its temporary buffer), 4:1:1 JPEGs to NV12 (upstream reads
/// outside the chroma planes) and progressive JPEGs to NV12.
#[test]
fn mjpg_conversions_upstream_mishandles() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/video/testdata/images");
    let convert = |name: &str, f: PixelFormat, pitch: i32, size: usize| {
        let data = std::fs::read(format!("{dir}/{name}")).unwrap();
        let mut dst = vec![0u8; size];
        crate::video::surface::convert_pixels(
            37,
            23,
            PixelFormat::MJPG,
            &data,
            data.len() as i32,
            f,
            &mut dst,
            pitch,
        )
        .map(|()| dst)
    };

    // MJPG to YUY2 decodes to an NV12 buffer with the YUY2 pitch first (the
    // step upstream overflows), then fails converting it, as YV12 does
    let nv12_size = 37 * 23 + 19 * 2 * 12;
    let nv12 = convert("j420.jpg", PixelFormat::NV12, 37, nv12_size).unwrap();
    assert!(nv12.iter().any(|&b| b != 0));
    let e = convert("j420.jpg", PixelFormat::YUY2, 76, 76 * 23).unwrap_err();
    assert_eq!(
        e.message(),
        "SDL_ConvertPixels_YUV_to_YUV: colorspace conversion not supported"
    );

    // 4:1:1 to NV12 is refused; progressive files decode
    let e = convert("j411.jpg", PixelFormat::NV12, 37, nv12_size).unwrap_err();
    assert_eq!(e.message(), "Unexpected chroma subsampling");
    assert!(convert("jprog.jpg", PixelFormat::NV12, 37, nv12_size).is_ok());
    assert!(convert("jgrayprog.jpg", PixelFormat::NV12, 37, nv12_size).is_ok());
    // A destination too small for the NV12 planes is an error, not a write past it
    let e = convert("j420.jpg", PixelFormat::NV12, 37, 100).unwrap_err();
    assert_eq!(e.message(), "Parameter 'dst' is invalid");
}

#[test]
fn save_and_load_files() {
    let dir = crate::test_support::TempDir::new("image");
    let path = std::path::Path::new(&dir.0).join("out.png");
    let mut s = Surface::new(5, 3, PixelFormat::RGBA32).unwrap();
    s.pixels_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
        .for_each(|(i, b)| *b = i as u8);
    s.save_png(&path).unwrap();
    let back = Surface::load_png(&path).unwrap();
    assert_eq!(
        (back.width(), back.height(), back.format()),
        (5, 3, PixelFormat::RGBA32)
    );
    assert_eq!(surf_hash(&back), surf_hash(&s));
    let any = Surface::load(&path).unwrap();
    assert_eq!(surf_hash(&any), surf_hash(&s));

    let mut nopal = Surface::new(2, 2, PixelFormat::INDEX4MSB).unwrap();
    nopal.set_palette(None).unwrap();
    assert_eq!(
        nopal
            .save_png(std::path::Path::new(&dir.0).join("x.png"))
            .unwrap_err()
            .message(),
        "Indexed surfaces must have a palette"
    );
    assert!(Surface::load_jpg(std::path::Path::new(&dir.0).join("missing.jpg")).is_err());
}

#[test]
fn wide_and_tall_png_round_trip() {
    // 65536 pixels and more in either direction need all four bytes of the
    // IHDR width and height.
    for (w, h) in [(70000, 2), (2, 70000), (65536, 1)] {
        let mut s = Surface::new(w, h, PixelFormat::RGBA32).unwrap();
        s.pixels_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
            .for_each(|(i, b)| *b = (i * 7) as u8);
        let mut io = IoStream::from_dynamic_mem();
        s.save_png_io(&mut io).unwrap();
        let png = io.dynamic_memory().unwrap().to_vec();
        assert_eq!(
            &png[16..24],
            &[(w as u32).to_be_bytes(), (h as u32).to_be_bytes()].concat()
        );
        let back = Surface::load_png_io(&mut IoStream::from_const_mem(&png)).unwrap();
        assert_eq!(
            (back.width(), back.height(), back.format()),
            (w, h, PixelFormat::RGBA32)
        );
        assert_eq!(surf_hash(&back), surf_hash(&s));
    }
}
