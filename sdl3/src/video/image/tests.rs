// Tests of the image codecs: a session replayed from a C harness built
// against upstream SDL (its stb_image and miniz), whose output is
// testdata/image_trace.txt. Its NV12 output reads the component planes
// with their padded stride (`w2`), as the translation does. The images in testdata/images come from
// tools/gen_png_testdata.py (PNG) and ImageMagick (JPEG).

use std::fmt::Write as _;

use crate::io::{IoStream, IoWhence};
use crate::video::pixels::{Color, PixelFormat};
use crate::video::surface::{read_palette, write_palette, Surface};

const FNV0: u64 = 14695981039346656037;

/// `(name, bytes)` of each of the files in testdata/images.
macro_rules! images {
    [$($name:literal),* $(,)?] => {
        &[$(($name, include_bytes!(concat!("../testdata/images/", $name)) as &[u8])),*]
    };
}

/// The images of testdata/images, in name order, embedded: the tests of
/// the Emscripten build see only their own (in-memory) file system.
const IMAGES: &[(&str, &[u8])] = images![
    "ancillary.png",
    "bad_depth.png",
    "bad_filter.png",
    "bad_zlib.png",
    "best.png",
    "fixed.png",
    "g1.png",
    "g16.png",
    "g16trns.png",
    "g1i.png",
    "g2.png",
    "g2f0.png",
    "g2f1.png",
    "g2f2.png",
    "g2f3.png",
    "g2f4.png",
    "g4.png",
    "g4trns.png",
    "g8.png",
    "g8i5x1.png",
    "g8trns.png",
    "ga16.png",
    "ga8.png",
    "ga8i3x3.png",
    "j100.jpg",
    "j411.jpg",
    "j420.jpg",
    "j422.jpg",
    "j440.jpg",
    "j444.jpg",
    "j64.jpg",
    "jcmyk.jpg",
    "jgray.jpg",
    "jgrayprog.jpg",
    "jone.jpg",
    "jprog.jpg",
    "jrst.jpg",
    "no_idat.png",
    "one.png",
    "p1.png",
    "p2.png",
    "p4.png",
    "p4i.png",
    "p4key2.png",
    "p8.png",
    "p8key.png",
    "p8trns.png",
    "rgb16.png",
    "rgb16trns.png",
    "rgb8.png",
    "rgb8f0.png",
    "rgb8f1.png",
    "rgb8f2.png",
    "rgb8f3.png",
    "rgb8f4.png",
    "rgb8i.png",
    "rgb8i1x1.png",
    "rgb8trns.png",
    "rgba16.png",
    "rgba16f0.png",
    "rgba16f1.png",
    "rgba16f2.png",
    "rgba16f3.png",
    "rgba16f4.png",
    "rgba16i.png",
    "rgba8.png",
    "rle.png",
    "split.png",
    "stored.png",
    "tall.png",
    "truncated.png",
    "unknown_critical.png",
    "wide.png",
    "wide1bit.png",
];

/// [`IMAGES`] written to a temporary directory, for the loaders that read
/// a path.
fn image_dir() -> crate::test_support::TempDir {
    let dir = crate::test_support::TempDir::new("images");
    for (name, data) in IMAGES {
        std::fs::write(dir.path(name), data).unwrap();
    }
    dir
}

/// [`IMAGES`] has every file of testdata/images.
#[test]
#[cfg_attr(target_os = "emscripten", ignore = "host file system")]
fn images_are_the_testdata_files() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/video/testdata/images");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    let embedded: Vec<&str> = IMAGES.iter().map(|&(name, _)| name).collect();
    assert_eq!(names, embedded);
}

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
    let dir = image_dir();
    for (name, _) in IMAGES {
        let path = dir.path(name);
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
        // planes)
        if let Ok(s) = &s {
            if is_jpg && !name.contains("411") {
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
    // The trace comes from upstream built for x86-64, whose SSE2 YUV to RGB
    // kernel differs from the portable one an AArch64 build uses: pin the
    // x86 selection so every architecture checks the same kernels.
    let x86 = crate::video::blit::SimdSupport {
        mmx: true,
        sse: true,
        sse2: true,
        sse41: true,
        avx2: true,
        neon: false,
    };
    crate::video::blit::SIMD_OVERRIDE.with(|o| o.set(Some(x86)));
    let out = session();
    crate::video::blit::SIMD_OVERRIDE.with(|o| o.set(None));
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
/// overflows its temporary buffer) and 4:1:1 JPEGs to NV12 (upstream reads
/// outside the chroma planes).
#[test]
fn mjpg_conversions_upstream_mishandles() {
    let convert = |name: &str, f: PixelFormat, pitch: i32, size: usize| {
        let &(_, data) = IMAGES.iter().find(|&&(n, _)| n == name).unwrap();
        let mut dst = vec![0u8; size];
        crate::video::surface::convert_pixels(
            37,
            23,
            PixelFormat::MJPG,
            data,
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
fn mjpg_to_nv12_keeps_rows_aligned() {
    // A 37-pixel-wide grayscale JPEG: its plane rows are padded to 40 bytes,
    // and the NV12 Y plane holds the same luma rows as the decoded image
    let dir = image_dir();
    for name in ["jgray.jpg", "jgrayprog.jpg"] {
        let path = dir.path(name);
        let gray = Surface::load_jpg(&path).unwrap();
        assert_eq!(gray.format(), PixelFormat::INDEX8);
        let (w, h) = (gray.width() as usize, gray.height() as usize);
        assert_eq!(w, 37);
        let data = std::fs::read(&path).unwrap();
        let pixels = gray.raw_pixels().unwrap();
        let gray_pitch = gray.pitch() as usize;
        for pitch in [w, w + 1] {
            let mut nv12 = vec![0u8; pitch * h + pitch.div_ceil(2) * 2 * h.div_ceil(2)];
            crate::video::surface::convert_pixels(
                w as i32,
                h as i32,
                PixelFormat::MJPG,
                &data,
                data.len() as i32,
                PixelFormat::NV12,
                &mut nv12,
                pitch as i32,
            )
            .unwrap();
            for y in 0..h {
                assert_eq!(
                    &nv12[y * pitch..][..w],
                    &pixels[y * gray_pitch..][..w],
                    "{name} pitch {pitch} row {y}"
                );
            }
        }
    }
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
