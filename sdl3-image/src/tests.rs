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
            let detectors: [(&str, fn(&mut IoStream<'_>) -> bool); 15] = [
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
            let detectors: [(&str, fn(&mut IoStream<'_>) -> bool); 3] = [
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
                    Ok(mut c) => dump_saved(save_with(&mut c, |s, io| crate::save_tga_io(s, io))),
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
