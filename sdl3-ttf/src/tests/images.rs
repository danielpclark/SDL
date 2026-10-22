// Image decoding tests of sdl3-ttf's PlutoVG against PlutoVG's C and its
// copy of stb_image.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The images in `testdata/images`, made by
//! `tools/gen_sdl_ttf_testdata.py --svg`, are small BMP, GIF, HDR, PIC,
//! PNM, PSD and TGA images covering stb_image's loaders, and broken ones.
//! `testdata/image_reference.txt` is the output of a C program that
//! decodes each of them, four truncated copies and sixteen copies with a
//! byte changed (picked by the same generator as below), with PlutoVG's
//! copy of stb_image (`stbi_load_from_memory()` to RGBA) and with
//! `plutovg_surface_load_from_image_data()` from the PlutoVG library
//! SDL_ttf's CMake builds. Decodes whose output C computes from
//! uninitialized memory (it differs between runs with different malloc
//! fillings, or with automatic variables filled with a pattern: palette
//! entries a BMP doesn't set, the pixels of uncompressed TGA and flat HDR
//! images that stbi__getn() can't read whole) read "undefined" there: they
//! are run (and mustn't panic) but not compared.

use super::{fnv, FNV0};
use crate::plutovg::plutovg_surface_load_from_image_data;
use sdl3::video::stb_image::load_from_memory_all_formats;

/// `(name, bytes)` of each of the files in testdata/images.
macro_rules! images {
    [$($name:literal),* $(,)?] => {
        &[$(($name, include_bytes!(concat!("../testdata/images/", $name)) as &[u8])),*]
    };
}

/// The images of testdata/images, in name order, embedded: the tests of
/// the Emscripten build see only their own (in-memory) file system.
const IMAGES: &[(&str, &[u8])] = images![
    "bmp1.bmp",
    "bmp16.bmp",
    "bmp16bf.bmp",
    "bmp16v3.bmp",
    "bmp24.bmp",
    "bmp24td.bmp",
    "bmp32.bmp",
    "bmp32v4.bmp",
    "bmp32v5.bmp",
    "bmp32v5a0.bmp",
    "bmp4.bmp",
    "bmp8.bmp",
    "bmp8os2.bmp",
    "bmpbadmask.bmp",
    "bmprle.bmp",
    "gif87.gif",
    "gifbg.gif",
    "gifbig.gif",
    "giflocal.gif",
    "gifnoimage.gif",
    "gifnotable.gif",
    "gifoutside.gif",
    "hdrbadsize.hdr",
    "hdrflat.hdr",
    "hdrnoformat.hdr",
    "hdrnotrle.hdr",
    "hdrrle.hdr",
    "hdrrle2.hdr",
    "pic.pic",
    "picbad.pic",
    "picrle.pic",
    "pnm5.pgm",
    "pnm5x16.pgm",
    "pnm6.ppm",
    "pnm6x16.ppm",
    "pnmmaxv.pgm",
    "pnmzero.pgm",
    "psd1.psd",
    "psd16.psd",
    "psd3.psd",
    "psd4.psd",
    "psd5.psd",
    "psdgrey.psd",
    "psdrle.psd",
    "tga16.tga",
    "tga24.tga",
    "tga32td.tga",
    "tgagrey.tga",
    "tgagreya.tga",
    "tganopal.tga",
    "tgapal.tga",
    "tgapal16.tga",
    "tgapalidx16.tga",
    "tgarle24.tga",
    "tgarlegrey.tga",
];

/// [`IMAGES`] has every file of testdata/images.
#[test]
#[cfg_attr(target_os = "emscripten", ignore = "host file system")]
fn images_are_the_testdata_files() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/testdata/images");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    let embedded: Vec<&str> = IMAGES.iter().map(|&(name, _)| name).collect();
    assert_eq!(names, embedded);
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

fn run(out: &mut Vec<String>, label: &str, d: &[u8]) {
    out.push(match load_from_memory_all_formats(d, 4) {
        Ok(i) => format!(
            "{label} rgba: {}x{} c={} {:016x}",
            i.x,
            i.y,
            i.comp,
            fnv(FNV0, &i.data)
        ),
        Err(_) => format!("{label} rgba: null"),
    });
    out.push(match plutovg_surface_load_from_image_data(d) {
        Some(s) => format!(
            "{label} surface: {}x{} stride={} {:016x}",
            s.width,
            s.height,
            s.stride,
            fnv(FNV0, &s.data[..(s.height * s.stride) as usize])
        ),
        None => format!("{label} surface: null"),
    });
}

#[test]
fn image_decoding_matches_upstream_reference() {
    let mut out = Vec::new();
    let mut rng = Lcg(7);
    for &(name, d) in IMAGES {
        let n = d.len();
        run(&mut out, name, d);
        for k in 1..=4 {
            let cut = n * k / 5;
            run(&mut out, &format!("{name} cut {cut}"), &d[..cut]);
        }
        for k in 0..16 {
            let mut m = d.to_vec();
            let pos = (rng.rnd() % n as u32) as usize;
            if k < 12 {
                m[pos] ^= 1 << (rng.rnd() % 8);
            } else {
                m[pos] = rng.rnd() as u8;
            }
            run(
                &mut out,
                &format!("{name} mut {k} at {pos} {:02x}", m[pos]),
                &m,
            );
        }
    }

    let reference: Vec<&str> = include_str!("../testdata/image_reference.txt")
        .lines()
        .collect();
    let mut failures = Vec::new();
    for (i, expected) in reference.iter().enumerate() {
        let actual = out.get(i).map(String::as_str).unwrap_or("<missing>");
        let undefined = expected
            .strip_suffix(": undefined")
            .is_some_and(|label| actual.starts_with(label));
        if actual != *expected && !undefined {
            failures.push(format!(
                "line {}:\n  expected {expected}\n  actual   {actual}",
                i + 1
            ));
        }
    }
    if out.len() > reference.len() {
        failures.push(format!("{} extra lines", out.len() - reference.len()));
    }
    assert!(
        failures.is_empty(),
        "{} of {} lines differ from upstream:\n{}",
        failures.len(),
        reference.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
