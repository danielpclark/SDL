// Tests of sdl3-image against upstream SDL_image's C.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The images in `testdata/images/` are upstream SDL_image's test images
//! (`test/`, under SDL_image's zlib license) and small synthetic ones made
//! by `tools/gen_sdl_image_testdata.py`. `testdata/reference.txt` is the
//! output of a C program built from upstream SDL_image (with its stb_image,
//! tiny_jpeg and QOI codecs) and SDL3: for every image, the detectors that
//! accept it, the surfaces loaded from the file, from memory and by type,
//! from truncations and from a corrupted copy, and the files saved from it
//! in every format (with their size and hash, and the surface reloaded from
//! them). Surfaces are described by their size, format, pitch and FNV-1a
//! hashes of their pixel rows and palette, their color key, blend mode,
//! hotspot and alternate images.
//!
//! Where upstream's results can't be had here, the comparison says why:
//! formats not translated yet load as "Unsupported image format", and the
//! GIFs whose header the GIF decoder rejects (which upstream loads through
//! an endless recursion, until it crashes or runs out of memory) fail with
//! the decoder's error.

use sdl3::io::IoStream;
use sdl3::video::{
    PixelFormat, Surface, PROP_SURFACE_HOTSPOT_X_NUMBER, PROP_SURFACE_HOTSPOT_Y_NUMBER,
};

use crate::qoi::codec;

macro_rules! images {
    ($($name:literal,)*) => {
        &[$(($name, include_bytes!(concat!("testdata/images/", $name)) as &[u8]),)*]
    };
}

static IMAGES: &[(&str, &[u8])] = images![
    "cur_multi.cur",
    "gif_anim.gif",
    "gif_interlace.gif",
    "gif_offset.gif",
    "gif_plain.gif",
    "gif_trans.gif",
    "ico_24.ico",
    "ico_8.ico",
    "ico_mono.ico",
    "ico_multi.ico",
    "ico_pal.ico",
    "ico_png.ico",
    "jpg420.jpg",
    "jpg444.jpg",
    "jpgcmyk.jpg",
    "jpggray.jpg",
    "jpgprog.jpg",
    "lbm_pbm.lbm",
    "p1.pbm",
    "p2.pgm",
    "p3.ppm",
    "p3d5.ppm",
    "p4.pbm",
    "p5.pgm",
    "p5d4.pgm",
    "p6.ppm",
    "palette.bmp",
    "palette.gif",
    "pcx1.pcx",
    "pcx24.pcx",
    "pcx2planes.pcx",
    "pcx4planes.pcx",
    "pcx8.pcx",
    "png_gray.png",
    "png_graya.png",
    "png_interlace.png",
    "png_pal.png",
    "png_palalpha.png",
    "png_palkey.png",
    "png_rgba16.png",
    "rgbrgb.ani",
    "rgbrgb.avifs",
    "rgbrgb.gif",
    "rgbrgb.png",
    "rgbrgb.webp",
    "sample.avif",
    "sample.bmp",
    "sample.cur",
    "sample.ico",
    "sample.jpg",
    "sample.jxl",
    "sample.pcx",
    "sample.png",
    "sample.pnm",
    "sample.qoi",
    "sample.tga",
    "sample.tif",
    "sample.webp",
    "sample.xcf",
    "sample.xpm",
    "svg-class.svg",
    "svg.svg",
    "tga16.tga",
    "tga24.tga",
    "tga24rle.tga",
    "tga32.tga",
    "tga32rle.tga",
    "tga8.tga",
    "tga8rle.tga",
    "tgacmap15.tga",
    "tgacmap32.tga",
    "tgagrey.tga",
    "tgagreyrle.tga",
    "thumb.xv",
];

/// An `is_*` function.
type Detector = fn(&mut IoStream<'_>) -> bool;

fn image(name: &str) -> &'static [u8] {
    IMAGES
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no test image {name}"))
        .1
}

const FNV0: u64 = 14695981039346656037;

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn surface_hash(s: &Surface<'_>) -> u64 {
    let mut h = FNV0;
    let format = s.format();
    let mut row = s.width() as usize * format.bytes_per_pixel() as usize;
    if format.bits_per_pixel() < 8 {
        row = (s.width() as usize * format.bits_per_pixel() as usize).div_ceil(8);
    }
    let Some(pixels) = s.pixels() else {
        return h;
    };
    let pitch = s.pitch() as usize;
    for y in 0..s.height() as usize {
        h = fnv(h, &pixels[y * pitch..y * pitch + row]);
    }
    h
}

/// The harness's `describe1()`.
fn describe1(s: &mut Surface<'_>) -> String {
    let mut out = format!(
        "{}x{} {} pitch={} hash={:016x}",
        s.width(),
        s.height(),
        s.format().name(),
        s.pitch(),
        surface_hash(s)
    );
    if let Some(p) = s.palette() {
        let p = p.read().unwrap();
        let bytes: Vec<u8> = p
            .colors()
            .iter()
            .flat_map(|c| [c.r, c.g, c.b, c.a])
            .collect();
        out += &format!(" pal={}:{:016x}", p.len(), fnv(FNV0, &bytes));
    }
    if let Some(key) = s.color_key() {
        out += &format!(" key={key}");
    }
    out += &format!(" blend={}", s.blend_mode().0);
    let props = s.properties();
    if props.contains(PROP_SURFACE_HOTSPOT_X_NUMBER) {
        out += &format!(
            " hot={},{}",
            props
                .get_number(PROP_SURFACE_HOTSPOT_X_NUMBER)
                .unwrap_or(-1),
            props
                .get_number(PROP_SURFACE_HOTSPOT_Y_NUMBER)
                .unwrap_or(-1)
        );
    }
    out
}

/// The harness's `describe()`.
fn describe(s: sdl3::Result<Surface<'_>>) -> String {
    match s {
        Err(e) => format!("err: {e}"),
        Ok(mut s) => {
            let mut out = describe1(&mut s);
            for alt in s.alternate_images_mut() {
                out += " | alt ";
                out += &describe1(alt);
            }
            out
        }
    }
}

fn load_mem(data: &[u8], type_: Option<&str>) -> String {
    let mut io = IoStream::from_const_mem(data);
    describe(crate::load_typed_io(&mut io, type_))
}

fn ext_of(name: &str) -> Option<&str> {
    name.rfind('.').map(|i| &name[i + 1..])
}

/// The harness's `dump_saved()`.
fn dump_saved(saved: sdl3::Result<IoStream<'_>>) -> String {
    match saved {
        Err(e) => format!("err: {e}"),
        Ok(io) => {
            let bytes = io.dynamic_memory().unwrap_or(&[]).to_vec();
            format!(
                "bytes={} hash={:016x} | reload: {}",
                bytes.len(),
                fnv(FNV0, &bytes),
                load_mem(&bytes, Some("TGA"))
            )
        }
    }
}

fn save_with(
    surface: &mut Surface<'_>,
    save: impl FnOnce(&mut Surface<'_>, &mut IoStream<'_>) -> sdl3::Result<()>,
) -> sdl3::Result<IoStream<'static>> {
    let mut io = IoStream::from_dynamic_mem();
    save(surface, &mut io).map(|()| io)
}

/// Formats SDL_image doesn't decode in this crate yet, which upstream's
/// harness loaded.
fn untranslated(name: &str) -> bool {
    matches!(ext_of(name), Some("svg" | "xpm" | "xcf" | "lbm" | "xv"))
}

/// Compare a result with the reference, allowing for what upstream can't
/// do the same way (see the module documentation).
fn check(name: &str, label: &str, expected: &str, actual: &str, failures: &mut Vec<String>) {
    let ok = if expected == actual {
        true
    } else if ext_of(name) == Some("gif")
        && (expected == "crash"
            || expected == "err: Out of memory"
            || expected.starts_with("err: Failed to create properties for anima"))
    {
        // upstream's endless recursion on a GIF the GIF decoder rejects
        actual.starts_with("err: ")
    } else if expected == "err: " {
        // upstream leaves the error message empty (or stale) here
        actual.starts_with("err: ")
    } else if let Some(unstable) = expected.strip_prefix("unstable ") {
        // upstream decodes uninitialized memory (stb_image on a truncated
        // progressive JPEG): its pixels vary from run to run, here they are 0
        let shape = |s: &str| s.split(" hash=").next().unwrap_or("").to_owned();
        !actual.starts_with("err: ") && shape(unstable) == shape(actual)
    } else if untranslated(name) {
        actual == "err: Unsupported image format"
    } else {
        false
    };
    if !ok {
        failures.push(format!(
            "{name}: {label}:\n  expected {expected}\n  actual   {actual}"
        ));
    }
}

#[test]
fn matches_upstream_reference() {
    let reference = include_str!("testdata/reference.txt");
    let tmp = std::env::temp_dir().join(format!("sdl3-image-test-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();

    let mut failures = Vec::new();
    let mut checked = 0;
    let mut name = "";
    let mut data: &[u8] = &[];
    let mut loaded: Option<Surface<'static>> = None;
    let tga_formats = [
        PixelFormat::INDEX8,
        PixelFormat::RGB24,
        PixelFormat::BGR24,
        PixelFormat::RGBA32,
        PixelFormat::BGRA32,
        PixelFormat::XRGB8888,
        PixelFormat::XRGB1555,
        PixelFormat::ARGB1555,
        PixelFormat::ARGB8888,
    ];
    let mut tga_index = 0;

    for line in reference.lines() {
        if let Some(n) = line.strip_prefix("== ") {
            name = n;
            data = image(name);
            let path = tmp.join(name);
            std::fs::write(&path, data).unwrap();
            loaded = crate::load(&path).ok();
            tga_index = 0;
            continue;
        }
        let (label, expected) = line
            .split_once(": ")
            .unwrap_or((line.trim_end_matches(':'), ""));
        let actual = if label == "is" {
            let mut io = IoStream::from_const_mem(data);
            let mut found = String::new();
            // (upstream's harness is built without libavif, libjxl,
            // libtiff and libwebp, so their detectors are on the is_extra
            // line, and AVIF's is tested separately)
            let detectors: [(&str, Detector); 15] = [
                ("ANI", crate::is_ani),
                ("CUR", crate::is_cur),
                ("BMP", crate::is_bmp),
                ("GIF", crate::is_gif),
                ("ICO", crate::is_ico),
                ("JPG", crate::is_jpg),
                ("LBM", crate::is_lbm),
                ("PCX", crate::is_pcx),
                ("PNG", crate::is_png),
                ("PNM", crate::is_pnm),
                ("QOI", crate::is_qoi),
                ("SVG", crate::is_svg),
                ("XCF", crate::is_xcf),
                ("XPM", crate::is_xpm),
                ("XV", crate::is_xv),
            ];
            for (n, is) in detectors {
                if is(&mut io) {
                    found += " ";
                    found += n;
                }
                assert_eq!(io.tell().unwrap(), 0, "{name}: is_{n} moved the stream");
            }
            // (the C line is "is:" followed by the names)
            let expected = line.strip_prefix("is:").unwrap();
            check(name, label, expected, &found, &mut failures);
            checked += 1;
            continue;
        } else if label == "is_extra" {
            let mut io = IoStream::from_const_mem(data);
            let mut found = String::new();
            let detectors: [(&str, Detector); 3] = [
                ("TIF", crate::is_tif),
                ("JXL", crate::is_jxl),
                ("WEBP", crate::is_webp),
            ];
            for (n, is) in detectors {
                if is(&mut io) {
                    found += " ";
                    found += n;
                }
                assert_eq!(io.tell().unwrap(), 0, "{name}: is_{n} moved the stream");
            }
            let expected = line.strip_prefix("is_extra:").unwrap();
            check(name, label, expected, &found, &mut failures);
            checked += 1;
            continue;
        } else if label == "load" {
            describe(crate::load(tmp.join(name)))
        } else if label == "load_io" {
            load_mem(data, None)
        } else if label == "typed" {
            load_mem(data, ext_of(name))
        } else if let Some(n) = label.strip_prefix("trunc ") {
            let n: usize = n.parse().unwrap();
            load_mem(&data[..n], ext_of(name))
        } else if label == "xor" {
            let mut x = data.to_vec();
            for i in (20..x.len()).step_by(3) {
                x[i] ^= 0x5A;
            }
            load_mem(&x, ext_of(name))
        } else {
            // the saving lines, from the surface IMG_Load() returned
            if untranslated(name) {
                continue;
            }
            let Some(surface) = loaded.as_mut() else {
                failures.push(format!("{name}: {label}: the image didn't load"));
                continue;
            };
            if let Some(t) = label.strip_prefix("save ") {
                dump_saved(save_with(surface, |s, io| crate::save_typed_io(s, io, t)))
            } else if let Some(q) = label.strip_prefix("save_jpg ") {
                let q: i32 = q.parse().unwrap();
                dump_saved(save_with(surface, |s, io| crate::save_jpg_io(s, io, q)))
            } else if let Some(f) = label.strip_prefix("save_tga ") {
                // the harness skips INDEX8 for the other formats
                while tga_formats[tga_index].name() != f {
                    tga_index += 1;
                }
                let format = tga_formats[tga_index];
                tga_index += 1;
                match surface.convert(format) {
                    Ok(mut c) => dump_saved(save_with(&mut c, crate::save_tga_io)),
                    Err(e) => format!("err: {e}"),
                }
            } else if let Some(ch) = label.strip_prefix("qoi_encode ") {
                let ch: u8 = ch.parse().unwrap();
                let format = if ch == 4 {
                    PixelFormat::RGBA32
                } else {
                    PixelFormat::RGB24
                };
                let c = surface.convert(format).unwrap();
                let (w, h, pitch) = (c.width() as usize, c.height() as usize, c.pitch() as usize);
                let pixels = c.pixels().unwrap();
                let mut packed = Vec::new();
                for y in 0..h {
                    packed.extend_from_slice(&pixels[y * pitch..y * pitch + w * ch as usize]);
                }
                let desc = codec::Desc {
                    width: w as u32,
                    height: h as u32,
                    channels: ch,
                    colorspace: codec::QOI_SRGB,
                };
                match codec::encode(&packed, &desc) {
                    Some(q) => format!(
                        "bytes={} hash={:016x} | reload: {}",
                        q.len(),
                        fnv(FNV0, &q),
                        load_mem(&q, None)
                    ),
                    None => "null".to_owned(),
                }
            } else {
                panic!("unknown reference line {line:?}");
            }
        };
        check(name, label, expected, &actual, &mut failures);
        checked += 1;
    }
    let _ = std::fs::remove_dir_all(&tmp);

    assert!(checked > 2000, "only {checked} reference lines");
    assert!(
        failures.is_empty(),
        "{} of {checked} results differ from upstream:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// A stream that can't seek, like a pipe.
struct Unseekable(&'static [u8]);

impl sdl3::io::IoInterface for Unseekable {
    fn can_read(&self) -> bool {
        true
    }
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, sdl3::io::IoStop> {
        let n = buf.len().min(self.0.len());
        buf[..n].copy_from_slice(&self.0[..n]);
        self.0 = &self.0[n..];
        Ok(n)
    }
}

#[test]
fn every_detector_on_every_image() {
    // Each detector accepts exactly the images of its format (the PNG inside
    // an icon is still an icon), and leaves the stream where it was.
    let detectors: [(&[&str], Detector); 19] = [
        (&["ani"], crate::is_ani),
        (&["avif", "avifs"], crate::is_avif),
        (&["bmp"], crate::is_bmp),
        (&["cur"], crate::is_cur),
        (&["gif"], crate::is_gif),
        (&["ico"], crate::is_ico),
        (&["jpg"], crate::is_jpg),
        (&["jxl"], crate::is_jxl),
        (&["lbm"], crate::is_lbm),
        (&["pcx"], crate::is_pcx),
        (&["png"], crate::is_png),
        (&["pbm", "pgm", "ppm", "pnm"], crate::is_pnm),
        (&["qoi"], crate::is_qoi),
        (&["svg"], crate::is_svg),
        (&["tif"], crate::is_tif),
        (&["webp"], crate::is_webp),
        (&["xcf"], crate::is_xcf),
        (&["xpm"], crate::is_xpm),
        (&["xv"], crate::is_xv),
    ];
    for (name, data) in IMAGES {
        let ext = ext_of(name).unwrap();
        // (at an offset, to check that the position is restored, not reset)
        let mut padded = vec![0u8; 3];
        padded.extend_from_slice(data);
        let mut io = IoStream::from_const_mem(&padded);
        for (exts, is) in detectors {
            io.seek(3, sdl3::io::IoWhence::Set).unwrap();
            assert_eq!(is(&mut io), exts.contains(&ext), "{name}: {exts:?}");
            assert_eq!(io.tell().unwrap(), 3, "{name}: {exts:?}");
        }
    }
}

#[test]
fn detectors_on_crafted_headers() {
    let is = |f: fn(&mut IoStream<'_>) -> bool, data: &[u8]| f(&mut IoStream::from_const_mem(data));
    assert!(is(crate::is_gif, b"GIF87a"));
    assert!(!is(crate::is_gif, b"GIF88a"));
    assert!(!is(crate::is_gif, b"GIF89"));
    assert!(is(crate::is_pnm, b"P1"));
    assert!(!is(crate::is_pnm, b"P7"));
    assert!(is(crate::is_tif, b"MM\0\x2a"));
    assert!(is(crate::is_jxl, b"\xff\x0a"));
    assert!(is(crate::is_webp, b"RIFF\0\0\0\0WEBPVP8L\0\0\0\0"));
    assert!(!is(crate::is_webp, b"RIFF\0\0\0\0WEBPVP8Z\0\0\0\0"));
    assert!(is(crate::is_svg, b"<?xml?>\n<svg>"));
    assert!(!is(crate::is_svg, b"<?xml?>\0<svg>"));
    assert!(is(crate::is_lbm, b"FORM\0\0\0\0ILBM"));
    assert!(is(crate::is_xcf, b"gimp xcf v011\0"));
    assert!(is(crate::is_xv, b"P7 332\n#END_OF_COMMENTS\n0 0 255\n"));
    assert!(!is(crate::is_xv, b"P7 332\n#BUILTIN:spade\n"));
    assert!(!is(crate::is_xv, b"P7 332\n#END_OF_COMMENTS\n-1 2\n"));
    assert!(is(crate::is_ico, b"\0\0\x01\0\x01\0"));
    assert!(!is(crate::is_ico, b"\0\0\x01\0\0\0"));
    assert!(is(crate::is_cur, b"\0\0\x02\0\x01\0"));
    assert!(is(crate::is_ani, b"RIFF\0\0\0\0ACON"));

    // AVIF: the file type box must name the avif or avis brand (libavif's
    // avifPeekCompatibleFileType())
    let ftyp = |size: u32, major: &[u8; 4], compatible: &[&[u8; 4]]| {
        let mut b = size.to_be_bytes().to_vec();
        b.extend_from_slice(b"ftyp");
        b.extend_from_slice(major);
        b.extend_from_slice(&[0, 0, 0, 0]);
        for c in compatible {
            b.extend_from_slice(*c);
        }
        b
    };
    assert!(is(crate::is_avif, &ftyp(16, b"avif", &[])));
    assert!(is(crate::is_avif, &ftyp(24, b"mif1", &[b"miaf", b"avis"])));
    assert!(!is(crate::is_avif, &ftyp(24, b"mif1", &[b"miaf", b"heic"])));
    // a brand list that isn't a multiple of 4, a box past the end, a size
    // smaller than the header
    let mut odd = ftyp(19, b"avif", &[]);
    odd.extend_from_slice(b"abc");
    assert!(!is(crate::is_avif, &odd));
    assert!(!is(crate::is_avif, &ftyp(17, b"avif", &[])));
    assert!(!is(crate::is_avif, &ftyp(4, b"avif", &[])));
    assert!(!is(crate::is_avif, &ftyp(0xFFFF_FFF0, b"avif", &[])));
    // a 64-bit size
    let mut large = 1u32.to_be_bytes().to_vec();
    large.extend_from_slice(b"ftyp");
    large.extend_from_slice(&24u64.to_be_bytes());
    large.extend_from_slice(b"avis\0\0\0\0");
    assert!(is(crate::is_avif, &large));
}

#[test]
fn front_end_errors() {
    // Unseekable streams are refused before any detection
    let mut io = IoStream::open(Unseekable(image("sample.png")));
    let e = crate::load_io(&mut io).unwrap_err();
    assert_eq!(e.to_string(), "Can't seek in this data source");

    // A TGA has no magic: it loads only when asked for by type
    let tga = image("sample.tga");
    let e = crate::load_io(&mut IoStream::from_const_mem(tga)).unwrap_err();
    assert_eq!(e.to_string(), "Unsupported image format");
    assert!(crate::load_typed_io(&mut IoStream::from_const_mem(tga), Some("tGa")).is_ok());

    // Formats with a magic number are detected whatever the type says, but
    // the magicless TGA comes first: asked for, it is tried first
    let png = image("sample.png");
    assert!(crate::load_typed_io(&mut IoStream::from_const_mem(png), Some("JPG")).is_ok());
    assert!(crate::load_typed_io(&mut IoStream::from_const_mem(png), Some("TGA")).is_err());

    // Formats not translated yet are unsupported, like an upstream build
    // without them
    for name in [
        "sample.webp",
        "sample.xpm",
        "svg.svg",
        "sample.tif",
        "sample.avif",
    ] {
        let e = crate::load_io(&mut IoStream::from_const_mem(image(name))).unwrap_err();
        assert_eq!(e.to_string(), "Unsupported image format", "{name}");
    }

    let e = crate::load("/nonexistent/sdl3-image/image.png").unwrap_err();
    assert!(!e.to_string().is_empty());
}

fn dump(save: impl FnOnce(&mut IoStream<'_>) -> sdl3::Result<()>) -> Vec<u8> {
    let mut io = IoStream::from_dynamic_mem();
    save(&mut io).unwrap();
    io.dynamic_memory().unwrap().to_vec()
}

#[test]
fn saving_through_files_and_types() {
    let tmp = std::env::temp_dir().join(format!("sdl3-image-save-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let mut surface = crate::load_io(&mut IoStream::from_const_mem(image("sample.png"))).unwrap();
    let (w, h) = (surface.width(), surface.height());

    for ext in ["bmp", "png", "tga", "jpg", "jpeg", "ico", "cur", "BMP"] {
        let path = tmp.join(format!("out.{ext}"));
        crate::save(&mut surface, &path).unwrap();
        let back = crate::load(&path).unwrap();
        assert_eq!((back.width(), back.height()), (w, h), "{ext}");
    }
    let path = tmp.join("direct.png");
    crate::save_png(&mut surface, &path).unwrap();
    crate::save_bmp(&mut surface, tmp.join("direct.bmp")).unwrap();
    crate::save_tga(&mut surface, tmp.join("direct.tga")).unwrap();
    crate::save_jpg(&mut surface, tmp.join("direct.jpg"), 30).unwrap();
    crate::save_ico(&mut surface, tmp.join("direct.ico")).unwrap();
    crate::save_cur(&mut surface, tmp.join("direct.cur")).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        dump(|io| crate::save_png_io(&mut surface, io))
    );

    let mut io = IoStream::from_dynamic_mem();
    for (t, message) in [
        ("gif", "SDL_image built without GIF save support"),
        ("webp", "SDL_image built without WEBP save support"),
        ("avif", "SDL_image built without AVIF save support"),
        ("xyz", "Unsupported image format"),
    ] {
        let e = crate::save_typed_io(&mut surface, &mut io, t).unwrap_err();
        assert_eq!(e.to_string(), message);
    }
    assert!(crate::save_typed_io(&mut surface, &mut io, "").is_err());
    let e = crate::save(&mut surface, tmp.join("noext")).unwrap_err();
    assert_eq!(e.to_string(), "Couldn't determine file type");

    // Indexed surfaces need a palette
    let mut indexed = Surface::new(4, 4, PixelFormat::INDEX8).unwrap();
    for t in ["bmp", "png", "tga", "ico", "cur"] {
        let e = crate::save_typed_io(&mut indexed, &mut io, t).unwrap_err();
        assert_eq!(e.to_string(), "Indexed surfaces must have a palette", "{t}");
    }
    // TGA has a fixed list of formats; a failed save rewinds
    let mut other = Surface::new(4, 4, PixelFormat::RGB565).unwrap();
    let start = io.tell().unwrap();
    assert!(crate::save_tga_io(&mut other, &mut io).is_err());
    assert_eq!(io.tell().unwrap(), start);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn icons_keep_their_images_and_hotspots() {
    // A cursor's 32x32 image is the surface; the others are alternates
    let mut cur =
        crate::load_cur_io(&mut IoStream::from_const_mem(image("cur_multi.cur"))).unwrap();
    assert_eq!((cur.width(), cur.height()), (32, 32));
    assert_eq!(cur.alternate_images().len(), 1);
    let props = cur.properties();
    assert_eq!(props.get_number(PROP_SURFACE_HOTSPOT_X_NUMBER), Some(9));
    assert_eq!(props.get_number(PROP_SURFACE_HOTSPOT_Y_NUMBER), Some(10));

    // Saved (as PNG images) and reloaded: the same sizes and hotspots
    let saved = dump(|io| crate::save_cur_io(&mut cur, io));
    let mut back = crate::load_io(&mut IoStream::from_const_mem(&saved)).unwrap();
    assert_eq!(back.alternate_images().len(), 1);
    let shape = |s: &mut Surface<'_>| {
        let d = describe1(s);
        (
            d.split(' ').next().unwrap().to_owned(),
            d.split(' ').next_back().unwrap().to_owned(),
        )
    };
    assert_eq!(
        shape(&mut back),
        ("32x32".to_owned(), "hot=9,10".to_owned())
    );
    assert_eq!(
        shape(&mut back.alternate_images_mut()[0]),
        ("16x16".to_owned(), "hot=3,4".to_owned())
    );

    // An icon's first image is the surface
    let ico = crate::load_ico_io(&mut IoStream::from_const_mem(image("ico_multi.ico"))).unwrap();
    assert_eq!((ico.width(), ico.height()), (16, 16));
    assert_eq!(ico.alternate_images()[0].width(), 32);

    // Wrong loader: the error names the format, and the stream is rewound
    let mut io = IoStream::from_const_mem(image("ico_multi.ico"));
    let e = crate::load_cur_io(&mut io).unwrap_err();
    assert_eq!(e.to_string(), "File is not a Windows CUR file");
    assert_eq!(io.tell().unwrap(), 0);
}

#[test]
fn qoi_round_trip() {
    let mut pixels = Vec::new();
    for i in 0..(37 * 11) {
        let v = (i * 7 % 251) as u8;
        // runs, small differences, luma differences and new colors
        pixels.extend_from_slice(&[
            v / 3,
            v / 3 + (i % 3) as u8,
            255 - v,
            if i % 17 == 0 { 0 } else { 255 },
        ]);
    }
    let desc = codec::Desc {
        width: 37,
        height: 11,
        channels: 4,
        colorspace: codec::QOI_LINEAR,
    };
    let encoded = codec::encode(&pixels, &desc).unwrap();
    let (decoded, back) = codec::decode(&encoded, encoded.len() as i32, 0).unwrap();
    assert_eq!(back, desc);
    assert_eq!(decoded, pixels);

    let surface = crate::load_qoi_io(&mut IoStream::from_const_mem(&encoded)).unwrap();
    assert_eq!(surface.format(), PixelFormat::RGBA32);
    assert_eq!(surface.pixels().unwrap()[..37 * 4], pixels[..37 * 4]);

    // Invalid descriptions and data
    assert!(codec::encode(
        &pixels,
        &codec::Desc {
            channels: 2,
            ..desc
        }
    )
    .is_none());
    assert!(codec::encode(&pixels, &codec::Desc { width: 0, ..desc }).is_none());
    assert!(codec::decode(&encoded, 21, 4).is_none());
    assert!(codec::decode(&encoded, encoded.len() as i32, 2).is_none());
    let mut bad = encoded.clone();
    bad[12] = 5; // channels
    assert!(codec::decode(&bad, bad.len() as i32, 0).is_none());
    let e = crate::load_qoi_io(&mut IoStream::from_const_mem(&bad)).unwrap_err();
    assert_eq!(e.to_string(), "Couldn't parse QOI image");
}

#[test]
fn malformed_input_errors_without_panicking() {
    // Every truncation of every small image, and single-byte corruptions of
    // the first 64 bytes of a few, load or fail cleanly (the reference test
    // compares a sample of these with upstream). Truncated files fail,
    // except where upstream decodes what it has (GIF frames, stb_image).
    for (name, data) in IMAGES {
        if data.len() > 4096 {
            continue;
        }
        let step = (data.len() / 40).max(1);
        let lengths = (0..data.len().min(64)).chain((64..data.len()).step_by(step));
        for n in lengths {
            let mut io = IoStream::from_const_mem(&data[..n]);
            let _ = crate::load_typed_io(&mut io, ext_of(name));
        }
    }
    for name in [
        "sample.pcx",
        "tga8rle.tga",
        "p3.ppm",
        "gif_trans.gif",
        "ico_pal.ico",
        "sample.qoi",
    ] {
        let data = image(name);
        for i in 0..64.min(data.len()) {
            if name.ends_with(".qoi") && (4..12).contains(&i) {
                // (a QOI image of up to 400 million pixels is valid: its
                // dimensions are left alone)
                continue;
            }
            for v in [0x00, 0x7f, 0xff] {
                let mut x = data.to_vec();
                x[i] = v;
                let _ = crate::load_typed_io(&mut IoStream::from_const_mem(&x), ext_of(name));
            }
        }
    }
    // Dimensions too large to allocate are errors: an icon of 16777215 by
    // 8388607 pixels, a TGA of 65535 by 65535 with no data
    let mut ico = image("ico_24.ico").to_vec();
    ico[22 + 4..22 + 8].copy_from_slice(&0xFF_FFFFi32.to_le_bytes());
    ico[22 + 8..22 + 12].copy_from_slice(&0xFF_FFFFi32.to_le_bytes());
    assert!(crate::load_io(&mut IoStream::from_const_mem(&ico)).is_err());
    let mut tga = image("tga32.tga")[..18].to_vec();
    tga[12..16].copy_from_slice(&[0xff; 4]);
    assert!(crate::load_tga_io(&mut IoStream::from_const_mem(&tga)).is_err());

    // Truncated headers are errors
    for (name, n) in [
        ("sample.pcx", 100),
        ("sample.tga", 10),
        ("p6.ppm", 5),
        ("sample.ico", 30),
        ("sample.qoi", 20),
        ("gif_plain.gif", 9),
        ("sample.png", 30),
    ] {
        let mut io = IoStream::from_const_mem(&image(name)[..n]);
        assert!(
            crate::load_typed_io(&mut io, ext_of(name)).is_err(),
            "{name}"
        );
    }
}

#[test]
fn textures_from_files_and_streams() {
    let target = Surface::new(64, 64, PixelFormat::ARGB8888).unwrap();
    let mut renderer = sdl3::render::Renderer::software(target).unwrap();

    let t = crate::load_texture_io(
        &mut renderer,
        &mut IoStream::from_const_mem(image("sample.png")),
    )
    .unwrap();
    assert_eq!(renderer.texture_size(t).unwrap(), (23.0, 42.0));
    let t = crate::load_texture_typed_io(
        &mut renderer,
        &mut IoStream::from_const_mem(image("tga8.tga")),
        Some("tga"),
    )
    .unwrap();
    assert_eq!(renderer.texture_size(t).unwrap(), (23.0, 13.0));

    let path = std::env::temp_dir().join(format!("sdl3-image-texture-{}.qoi", std::process::id()));
    std::fs::write(&path, image("sample.qoi")).unwrap();
    let t = crate::load_texture(&mut renderer, &path).unwrap();
    assert!(renderer.texture_size(t).is_ok());
    let _ = std::fs::remove_file(&path);

    assert!(crate::load_texture_io(&mut renderer, &mut IoStream::from_const_mem(b"nope")).is_err());
}
